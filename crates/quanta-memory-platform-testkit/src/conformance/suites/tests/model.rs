use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModelTicketState {
    Pending,
    Committed,
    Released,
}

pub(super) struct ModelTicket<T> {
    pub(super) target: T,
    pub(super) class_axis: usize,
    pub(super) owner_axis: usize,
    pub(super) kind: ChargeAxis,
    pub(super) bytes: u64,
    pub(super) state: ModelTicketState,
}

#[derive(Default)]
pub(super) struct IndependentAccountingModel {
    class_axis_bytes: [u64; 2],
    owner_axis_bytes: [u64; 2],
    group_axis_bytes: [u64; 2],
    kind_bytes: [u64; 3],
    // FUP-004: the model tracks bucket x kind from the operation trace
    // itself — never by asking the target — so a cross-bucket kind swap
    // that preserves every aggregate still diverges from the view.
    class_axis_kind_bytes: [[u64; 3]; 2],
    owner_axis_kind_bytes: [[u64; 3]; 2],
    live_ticket_count: usize,
    rebuild_in_progress: bool,
    grouped: bool,
}

impl IndependentAccountingModel {
    fn grouped() -> Self {
        Self { grouped: true, ..Self::default() }
    }

    pub(super) fn apply_reserve(
        &mut self,
        class_axis: usize,
        owner_axis: usize,
        kind: ChargeAxis,
        bytes: u64,
    ) {
        self.class_axis_bytes[class_axis] += bytes;
        self.owner_axis_bytes[owner_axis] += bytes;
        if self.grouped {
            self.group_axis_bytes[owner_axis] += bytes;
        }
        self.kind_bytes[kind_index(kind)] += bytes;
        self.class_axis_kind_bytes[class_axis][kind_index(kind)] += bytes;
        self.owner_axis_kind_bytes[owner_axis][kind_index(kind)] += bytes;
        self.live_ticket_count += 1;
    }

    pub(super) fn apply_release<T>(&mut self, ticket: &ModelTicket<T>) {
        self.class_axis_bytes[ticket.class_axis] -= ticket.bytes;
        self.owner_axis_bytes[ticket.owner_axis] -= ticket.bytes;
        if self.grouped {
            self.group_axis_bytes[ticket.owner_axis] -= ticket.bytes;
        }
        self.kind_bytes[kind_index(ticket.kind)] -= ticket.bytes;
        self.class_axis_kind_bytes[ticket.class_axis][kind_index(ticket.kind)] -= ticket.bytes;
        self.owner_axis_kind_bytes[ticket.owner_axis][kind_index(ticket.kind)] -= ticket.bytes;
        self.live_ticket_count -= 1;
    }

    pub(super) fn view(&self) -> AccountingView {
        let total = self.owner_axis_bytes.iter().sum();
        // Both targets charge every conformance reserve to the Request
        // context, so the model's whole kind vector sits in bucket 0.
        let context_kind_bytes = [self.kind_bytes, [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]];
        AccountingView {
            total_class_bytes: self.class_axis_bytes.iter().sum(),
            total_owner_bytes: total,
            total_context_bytes: total,
            context_axis_bytes: [total, 0, 0, 0, 0],
            class_axis_bytes: self.class_axis_bytes,
            owner_axis_bytes: self.owner_axis_bytes,
            group_axis_bytes: self.group_axis_bytes,
            class_kind_bytes: self.class_axis_kind_bytes,
            owner_kind_bytes: self.owner_axis_kind_bytes,
            context_kind_bytes,
            live_ticket_count: self.live_ticket_count,
            total_retained_payload_bytes: self.kind_bytes[0],
            total_retained_metadata_bytes: self.kind_bytes[1],
            total_live_scratch_bytes: self.kind_bytes[2],
            rebuild_in_progress: self.rebuild_in_progress,
        }
    }
}

fn kind_index(kind: ChargeAxis) -> usize {
    match kind {
        ChargeAxis::RetainedPayload => 0,
        ChargeAxis::RetainedMetadata => 1,
        ChargeAxis::LiveScratch => 2,
    }
}

pub(super) fn run_independent_trace<T: BrokerConformanceTarget>(target: &mut T) {
    let owners = ["owner:conformance-a", "owner:conformance-b"];
    let kinds =
        [ChargeAxis::RetainedPayload, ChargeAxis::RetainedMetadata, ChargeAxis::LiveScratch];
    let mut model = IndependentAccountingModel::default();
    let mut tickets: Vec<ModelTicket<T::Ticket>> = Vec::new();
    let mut seed = 0x9e37_79b9_u64;

    for step in 0..256u64 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let live =
            tickets.iter().filter(|ticket| ticket.state != ModelTicketState::Released).count();
        let action = if live < 8 { seed % 5 } else { 2 };
        match action {
            0 | 4 => {
                let class_axis = ((seed >> 8) & 1) as usize;
                let owner_axis = ((seed >> 9) & 1) as usize;
                let kind = kinds[((seed >> 10) % 3) as usize];
                let bytes = 1 + ((seed >> 16) % 31);
                let ticket = target
                    .do_reserve(class_axis, owners[owner_axis], kind, bytes)
                    .unwrap_or_else(|error| panic!("step {step} reserve failed: {error}"));
                model.apply_reserve(class_axis, owner_axis, kind, bytes);
                tickets.push(ModelTicket {
                    target: ticket,
                    class_axis,
                    owner_axis,
                    kind,
                    bytes,
                    state: ModelTicketState::Pending,
                });
            }
            1 => {
                if let Some(ticket) =
                    tickets.iter_mut().find(|ticket| ticket.state == ModelTicketState::Pending)
                {
                    target.do_commit(&ticket.target).expect("first commit");
                    ticket.state = ModelTicketState::Committed;
                    assert!(target.do_commit(&ticket.target).is_err(), "duplicate commit accepted");
                }
            }
            2 | 3 => {
                if let Some(index) =
                    tickets.iter().position(|ticket| ticket.state != ModelTicketState::Released)
                {
                    target.do_release(&tickets[index].target).expect("terminal release");
                    model.apply_release(&tickets[index]);
                    tickets[index].state = ModelTicketState::Released;
                    assert!(
                        target.do_release(&tickets[index].target).is_err(),
                        "stale release accepted"
                    );
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(
            target.view(),
            Ok(model.view()),
            "independent model divergence after trace step {step}"
        );
    }

    for ticket in &mut tickets {
        if ticket.state != ModelTicketState::Released {
            target.do_release(&ticket.target).expect("final drain");
            model.apply_release(ticket);
            ticket.state = ModelTicketState::Released;
        }
    }
    assert_eq!(model.view(), AccountingView::default());
    assert_eq!(target.view(), Ok(AccountingView::default()));
}

#[derive(Clone, Debug)]
enum ModelOperation {
    Reserve { class_axis: usize, owner_axis: usize, kind: ChargeAxis, bytes: u64 },
    Commit(usize),
    Release(usize),
    BeginRebuild,
    FinishRebuild,
    Observe(Option<u64>),
}

fn model_operation() -> impl Strategy<Value = ModelOperation> {
    let kind = prop_oneof![
        Just(ChargeAxis::RetainedPayload),
        Just(ChargeAxis::RetainedMetadata),
        Just(ChargeAxis::LiveScratch),
    ];
    prop_oneof![
        5 => (0usize..2, 0usize..2, kind, 0u64..=63).prop_map(
            |(class_axis, owner_axis, kind, bytes)| ModelOperation::Reserve {
                class_axis,
                owner_axis,
                kind,
                bytes,
            }
        ),
        1 => Just(ModelOperation::Reserve {
            class_axis: 0,
            owner_axis: 0,
            kind: ChargeAxis::RetainedPayload,
            bytes: u64::MAX,
        }),
        2 => (0usize..96).prop_map(ModelOperation::Commit),
        3 => (0usize..96).prop_map(ModelOperation::Release),
        1 => Just(ModelOperation::BeginRebuild),
        1 => Just(ModelOperation::FinishRebuild),
        2 => prop::option::of(0u64..=8_192).prop_map(ModelOperation::Observe),
    ]
}

fn run_generated_trace<T: BrokerConformanceTarget>(target: &mut T, operations: &[ModelOperation]) {
    let owners = ["owner:conformance-a", "owner:conformance-b"];
    let mut model = IndependentAccountingModel::grouped();
    let mut tickets: Vec<ModelTicket<T::Ticket>> = Vec::new();
    let mut model_fence: Option<T::RebuildFence> = None;

    for (step, operation) in operations.iter().enumerate() {
        match *operation {
            ModelOperation::Reserve { class_axis, owner_axis, kind, bytes } => {
                let before = target.view().expect("pre-reserve observation");
                let result = target.do_reserve(class_axis, owners[owner_axis], kind, bytes);
                if model.rebuild_in_progress || bytes == u64::MAX {
                    assert!(result.is_err(), "step {step}: denied reserve unexpectedly passed");
                    assert_eq!(target.view(), Ok(before), "step {step}: denial mutated state");
                } else {
                    let ticket = result.unwrap_or_else(|error| {
                        panic!("step {step}: admissible reserve failed: {error}")
                    });
                    model.apply_reserve(class_axis, owner_axis, kind, bytes);
                    tickets.push(ModelTicket {
                        target: ticket,
                        class_axis,
                        owner_axis,
                        kind,
                        bytes,
                        state: ModelTicketState::Pending,
                    });
                }
            }
            ModelOperation::Commit(slot) if !model.rebuild_in_progress && !tickets.is_empty() => {
                let index = slot % tickets.len();
                let ticket = &mut tickets[index];
                let result = target.do_commit(&ticket.target);
                if ticket.state == ModelTicketState::Pending {
                    result.unwrap_or_else(|error| {
                        panic!("step {step}: first commit failed: {error}")
                    });
                    ticket.state = ModelTicketState::Committed;
                } else {
                    assert!(result.is_err(), "step {step}: non-pending commit passed");
                }
            }
            ModelOperation::Release(slot) if !model.rebuild_in_progress && !tickets.is_empty() => {
                let index = slot % tickets.len();
                let result = target.do_release(&tickets[index].target);
                if tickets[index].state == ModelTicketState::Released {
                    assert!(result.is_err(), "step {step}: stale release passed");
                } else {
                    result.unwrap_or_else(|error| {
                        panic!("step {step}: terminal release failed: {error}")
                    });
                    model.apply_release(&tickets[index]);
                    tickets[index].state = ModelTicketState::Released;
                }
            }
            ModelOperation::BeginRebuild if !model.rebuild_in_progress => {
                model_fence = Some(
                    target
                        .do_begin_rebuild()
                        .unwrap_or_else(|error| panic!("step {step}: begin failed: {error}")),
                );
                model.rebuild_in_progress = true;
            }
            ModelOperation::FinishRebuild if model.rebuild_in_progress => {
                let fence =
                    model_fence.take().expect("model tracks the rebuild fence with its window");
                target
                    .do_finish_rebuild(fence)
                    .unwrap_or_else(|error| panic!("step {step}: finish failed: {error}"));
                model.rebuild_in_progress = false;
            }
            ModelOperation::Observe(resident_bytes) => {
                let advisory = target
                    .observe_allocator(&AllocatorObservation { resident_bytes })
                    .unwrap_or_else(|error| panic!("step {step}: observation failed: {error}"));
                assert_eq!(advisory.semantic_bytes, model.view().total_owner_bytes);
                assert_eq!(advisory.allocator_resident_bytes, resident_bytes);
            }
            _ => {}
        }
        assert_eq!(
            target.view(),
            Ok(model.view()),
            "independent model divergence after generated step {step}: {operation:?}"
        );
    }

    if model.rebuild_in_progress {
        let fence = model_fence.take().expect("model tracks the fence with its window");
        target.do_finish_rebuild(fence).expect("final rebuild close");
        model.rebuild_in_progress = false;
    }
    for ticket in &mut tickets {
        if ticket.state != ModelTicketState::Released {
            target.do_release(&ticket.target).expect("generated trace final drain");
            model.apply_release(ticket);
            ticket.state = ModelTicketState::Released;
        }
    }
    assert_eq!(model.view(), AccountingView::default());
    assert_eq!(target.view(), Ok(AccountingView::default()));
}

fn grouped_fake() -> FakeProcessBroker {
    FakeProcessBroker::honest()
        .with_resource_group("conformance-group-a", 6_000, vec!["owner:conformance-a".to_string()])
        .with_resource_group("conformance-group-b", 6_000, vec!["owner:conformance-b".to_string()])
}

fn grouped_live_broker() -> LiveProcessMemoryBroker {
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 4_000,
        hard_cap_bytes: 6_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(10_000_000)
        .with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: 4_000,
                hard_cap_bytes: 6_000,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        )
        .with_class_budget(
            MemoryClass::EvidenceCache,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: 4_000,
                hard_cap_bytes: 6_000,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        )
        .with_resource_group("conformance-group-a", group.clone())
        .with_resource_group("conformance-group-b", group)
        .with_owner_group("owner:conformance-a", "conformance-group-a")
        .with_owner_group("owner:conformance-b", "conformance-group-b");
    quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Ci,
            quanta_memory_platform_core::ExecutionBudget::default(),
            residency,
        ))
        .build()
        .expect("valid grouped residency")
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        max_shrink_iters: 8_192,
        ..ProptestConfig::default()
    })]

    #[test]
    fn property_trace_matches_independent_model(
        operations in prop::collection::vec(model_operation(), 1..96)
    ) {
        run_generated_trace(&mut grouped_fake(), &operations);
        run_generated_trace(&mut grouped_live_broker(), &operations);
    }
}
