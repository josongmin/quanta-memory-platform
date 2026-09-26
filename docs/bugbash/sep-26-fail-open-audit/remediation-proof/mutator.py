from pathlib import Path
import subprocess, tarfile, io, tempfile, json, os, re, hashlib, time
ROOT=Path('/Users/songmin/Documents/code-new/quanta-memory-platform')
DEST=Path(tempfile.mkdtemp(prefix='qmp-sep26-candidate-'))
archive=subprocess.check_output(['git','archive','HEAD'],cwd=ROOT)
with tarfile.open(fileobj=io.BytesIO(archive)) as tf:
 for member in tf.getmembers():
  assert not Path(member.name).is_absolute() and '..' not in Path(member.name).parts
  assert not member.issym() and not member.islnk(), member.name
 tf.extractall(DEST)
changed=subprocess.check_output(['git','diff','--name-only','-z'],cwd=ROOT).decode().split('\0')
extra=subprocess.check_output(['git','ls-files','--others','--exclude-standard','-z'],cwd=ROOT).decode().split('\0')
for rel in changed+extra:
 if rel and (ROOT/rel).is_file():
  target=DEST/rel; target.parent.mkdir(parents=True,exist_ok=True); target.write_bytes((ROOT/rel).read_bytes())
proof=DEST/'proof';proof.mkdir()
base=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
files={str(p.relative_to(DEST)):hashlib.sha256(p.read_bytes()).hexdigest() for p in DEST.rglob('*.rs') if '.git' not in p.parts}
(proof/'source-sha256.json').write_text(json.dumps(files,indent=2)+'\n')
(proof/'source.json').write_text(json.dumps({'base':base,'snapshot':str(DEST),'created':time.time()},indent=2)+'\n')
core='crates/quanta-memory-platform-core/src/'
kit='crates/quanta-memory-platform-testkit/src/'
fac='crates/quanta-memory-platform-facade/src/'
cases=[]
def add(key,path,edits,selector,package='core'):
 cases.append({'id':key,'path':path,'edits':edits,'selector':selector,'package':'quanta-memory-platform-'+package,'cargo_args':['--all-features','--lib']})
def source(path):return (DEST/path).read_text()
# Reintroduce the exact old fake algorithm while retaining the new owning regressions.
p=kit+'fake/broker.rs'; s=source(p); old=subprocess.check_output(['git','show','HEAD:'+p],cwd=ROOT,text=True)
def reclaim_body(t):
 start=t.index('    pub fn reclaim_once(');end=t.index('    /// Open the rebuild window',start);return t[start:end]
add('fake-reclaim-old-axis-subtraction',p,[(reclaim_body(s),reclaim_body(old))],'fake::broker::tests::reclaim_preserves_ticket_provenance_kind_axes_and_group_refunds','testkit')
p=fac+'payload_store/service.rs';s=source(p)
a=s.index('            if entry.pin_count > 0 {',s.index('    fn delete_if_unpinned'))
b=s.index('            let canonical_digest',a)
loss='''            if entry.pin_count > 0 {
                let pin_count = entry.pin_count;
                let removed = state.entries[index].take().expect("mutation live entry");
                std::mem::forget(removed);
                state.live_entries -= 1;
                return Err(MemoryError::InvalidState(format!("delete rejected: payload still pinned ({pin_count} dependents)")));
            }
'''
add('pin-entry-loss-with-debit-retained',p,[(s[a:b],loss)],'faults::scenarios::tests::pinned_delete_failure_is_contained_and_bytes_survive','testkit')
p=core+'domain/ledger.rs';s=source(p)
needle='''                .owner_usage
                .get(owner)
                .ok_or_else(|| MemoryError::InvalidState("release owner axis is missing".into()))?;'''
add('ledger-nested-owner-scan',p,[(needle,needle.replace('.get(owner)', '.iter().find_map(|(candidate, usage)| (candidate == owner).then_some(usage))'))],'domain::ledger::tests::single_release_preflight_is_bounded_independent_of_owner_cardinality')
a=s.index('        for ((owner, kind), bytes) in &owner_refunds {',s.index('let owner_probes'))
b=s.index('        for ((context, kind), bytes)',a)
scan='''        for (owner, usage) in &self.owner_usage {
            for kind in [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch] {
                if let Some(bytes) = owner_refunds.get(&(owner.clone(), kind)) {
                    usage.slot(kind).checked_sub(*bytes).ok_or_else(|| MemoryError::InvalidState("release exceeds charged owner slot".into()))?;
                }
            }
        }
'''
add('ledger-outer-owner-scan',p,[(s[a:b],scan)],'domain::ledger::tests::single_release_preflight_is_bounded_independent_of_owner_cardinality')
p=core+'api/control.rs'
add('allocator-driven-pressure',p,[('struct BrokerCore {','struct BrokerCore {\n    last_allocator_resident: Option<u64>,'),('reclaim_stats: ReclaimRuntimeStats::default(),','reclaim_stats: ReclaimRuntimeStats::default(),\n                last_allocator_resident: None,'),('let used = snapshot.checked_total_owner_bytes()?;','let used = core.last_allocator_resident.unwrap_or(snapshot.checked_total_owner_bytes()?);'),('''            let core = self.lock()?;
            let semantic_snapshot = core.ledger.snapshot();''','''            let mut core = self.lock()?;
            core.last_allocator_resident = snapshot.resident_bytes;
            let semantic_snapshot = core.ledger.snapshot();''')],'api::control::tests::pressure_classifies_from_semantic_usage')
p=core+'application/controller.rs';s=source(p)
needle='''u64::try_from(u128::from(used_bytes) * 100 / u128::from(self.global_hard_limit_bytes))
                .unwrap_or(u64::MAX)'''
for operation in ['wrapping','saturating']:
 add('pressure-percent-'+operation,p,[(needle,f'used_bytes.{operation}_mul(100) / self.global_hard_limit_bytes')],'application::controller::tests::extreme_limits_classify_percent_without_wrapping_or_saturating_product')
p=core+'application/scheduler.rs';s=source(p);a=s.index('    fn admits(&self, group_name:',s.index('impl ResourceGroupScheduler'));b=s.index('\n}\n',a)
part=s[a:b];new=part.replace('&self.fair_burst_shares,','&admission::precompute_fair_burst_shares(&self.groups, self.shared_burst_pool_bytes),')
add('scheduler-recomputes-static-shares',p,[(part,new)],'application::scheduler::tests::admission_reuses_policy_static_fair_share_table_at_high_group_cardinality')
p=core+'domain/config.rs'
add('joint-class-group-floor',p,[('total_group_reserved.checked_add(residency.headroom_bytes)','total_group_reserved.checked_add(total_reserved).and_then(|combined| combined.checked_add(residency.headroom_bytes))')],'domain::config::tests::resource_group_reserved_has_an_independent_global_floor')
for case in json.loads(Path('/tmp/qmp-facade-mutations.json').read_text())['mutations']:
 cases.append({**case,'edits':[(case['old'],case['new'])]})
p=kit+'fixtures/mod.rs'
add('kit-constant-passed',p,[('''        let mut broker = self.fake_broker();
        ConformanceSuite::new().run_broker_accounting(&mut broker)''','''        ConformanceVerdict::Passed''')],'fixtures::tests::testkit_runs_conformance_against_its_fake_and_passes','testkit')
p=kit+'conformance/suites.rs'
needle=''.join('    verdicts.extend('+name+');\n' for name in ['run_payload_store_slice(kit)','run_retained_map_slice(kit)','run_transcript_slice(kit)','run_spill_slice()'])
add('facade-suite-slices-removed',p,[(needle,'')],'fixtures::tests::testkit_runs_facade_store_suite_against_live_broker_and_passes','testkit')
p=core+'domain/ledger.rs'
for key,statement in [('ledger-pre-aggregation-scan','let _scanned: u64 = self.owner_usage.values().map(UsageBreakdown::total).sum();'),('ledger-preflight-snapshot-copy','let _copied = self.snapshot();')]:
 add(key,p,[('        let mut class_refunds = BTreeMap::<(MemoryClass, ChargeKind), u64>::new();','        '+statement+'\n        let mut class_refunds = BTreeMap::<(MemoryClass, ChargeKind), u64>::new();')],'domain::ledger::tests::single_release_preflight_is_bounded_independent_of_owner_cardinality')
p=core+'api/control.rs'
add('allocator-mutates-pressure-cache',p,[('            let core = self.lock()?;\n            let semantic_snapshot = core.ledger.snapshot();','            let mut core = self.lock()?;\n            core.controller.update_from_semantic(snapshot.resident_bytes.unwrap_or(0));\n            let semantic_snapshot = core.ledger.snapshot();')],'api::control::tests::pressure_classifies_from_semantic_usage')
p=fac+'revocable_store/service.rs'
add('revocable-child-stale-filter',p,[('.arg("revocable_store::service::tests::compound_measurement_and_drop_panic_does_not_abort_insert")','.arg("revocable_store::service::tests::nonexistent_mutation_child")')],'revocable_store::service::tests::compound_measurement_and_drop_panic_does_not_abort_insert','facade')
for c in cases:
 s=source(c['path'])
 for a,b in c['edits']:
  assert a != b, ('no-op mutation', c['id'])
  if s.count(a)!=1: raise RuntimeError((c['id'],s.count(a),a[:130]))
  s=s.replace(a,b,1)
(proof/'mutation-spec.json').write_text(json.dumps(cases,indent=2)+'\n')
Path('/tmp/qmp-sep26-current-candidate.txt').write_text(str(DEST))
print(DEST)
print('mutations',len(cases))
