//! Opt-in Linux syscall-failure rail for Redb finalize (DURABILITY-001 L2).
//!
//! This harness injects `EIO` into the first `pwrite64` or `fdatasync` after
//! the finalize transaction is fully staged and immediately before
//! `txn.commit()`. It then cold-reopens the same database and accepts only the
//! two whole transaction outcomes: the exact committed recovery record remains
//! recoverable, or the record is fully Finalized. This is syscall-error
//! injection on the host filesystem, not block-device failure or power loss.
//!
//! Run through `scripts/durability/run-emulated-linux.sh`; do not add this to
//! normal PR test rails. The harness leaves its trace logs and Redb files in
//! the reported temporary artifact directory.

#[cfg(target_os = "linux")]
use std::error::Error;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, Output};
#[cfg(target_os = "linux")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use quanta_memory_platform_facade::spill::backings::redb::{
    finalize_kill_hooks::{install, FinalizeKillPoint},
    RedbSpillBacking,
};
#[cfg(target_os = "linux")]
use quanta_memory_platform_facade::spill::service::{
    DurableRestoreTarget, ReconciledDurableSpillTarget,
};
#[cfg(target_os = "linux")]
use quanta_memory_platform_facade::spill::{
    DurableRecordId, DurableRecordResolution, DurableRestorePhase,
};

#[cfg(target_os = "linux")]
const RECORD_COUNT: u64 = 2;

#[cfg(target_os = "linux")]
fn payload(index: u64) -> Vec<u8> {
    let mut bytes = vec![0xA5; 32];
    bytes[..8].copy_from_slice(&index.to_le_bytes());
    bytes
}

#[cfg(target_os = "linux")]
fn seed(path: &Path) -> Result<(), Box<dyn Error>> {
    let backing = RedbSpillBacking::create(path)?;
    for index in 0..RECORD_COUNT {
        let id = backing.reserve_record_id()?;
        assert_eq!(id.get(), index);
        if let Err(error) = backing.commit_spill(id, &payload(index)) {
            return Err(format!("seed spill {index} failed: {error:?}").into());
        }
    }
    let candidate = backing.prepare_restore()?.ok_or("missing seed candidate")?;
    assert_eq!(candidate.sequence, 0);
    backing
        .commit_restore(&candidate.claim)
        .map_err(|error| format!("seed restore commit failed: {error:?}"))?;
    Ok(())
}

/// Child hook marker parsed from strace's `write(1, ...)` line. This hook
/// returns immediately, so the next matching storage syscall belongs to the
/// actual Redb finalize commit, not seed/recovery work.
#[cfg(target_os = "linux")]
fn mark_finalize_commit(_: FinalizeKillPoint, sequence: u64, generation: u64) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "L2_FINALIZE_BEGIN {sequence} {generation}").expect("write finalize marker");
    out.flush().expect("flush finalize marker");
}

#[cfg(target_os = "linux")]
fn worker(path: &Path) -> Result<(), Box<dyn Error>> {
    let backing = RedbSpillBacking::create(path)?;
    let recovery = backing.recover_active_restore()?.ok_or("missing committed recovery record")?;
    assert_eq!(recovery.candidate.sequence, 0);
    assert_eq!(recovery.phase, DurableRestorePhase::Committed);
    install(FinalizeKillPoint::BeforeCommit, mark_finalize_commit);
    backing.finalize_restore(&recovery.candidate.claim)?;
    println!("L2_FINALIZE_OK {}", recovery.candidate.sequence);
    Ok(())
}

#[cfg(target_os = "linux")]
fn syscall_ordinal_before_marker(trace: &str, syscall: &str) -> Result<usize, Box<dyn Error>> {
    let lines: Vec<_> = trace.lines().collect();
    let marker = lines
        .iter()
        .position(|line| line.contains("L2_FINALIZE_BEGIN"))
        .ok_or("strace trace did not observe the finalize marker")?;
    let before = lines[..marker].iter().filter(|line| line.contains(&format!("{syscall}(")));
    let ordinal = before.count() + 1;
    if !lines[marker + 1..].iter().any(|line| line.contains(&format!("{syscall}("))) {
        return Err(format!("no {syscall} syscall observed after finalize marker").into());
    }
    Ok(ordinal)
}

#[cfg(target_os = "linux")]
fn verify_injection_after_marker(
    trace: &str,
    syscall: &str,
    expected_ordinal: usize,
) -> Result<(), Box<dyn Error>> {
    let lines: Vec<_> = trace.lines().collect();
    let marker = lines
        .iter()
        .position(|line| line.contains("L2_FINALIZE_BEGIN"))
        .ok_or("fault trace did not observe the finalize marker")?;
    let injected = lines
        .iter()
        .position(|line| {
            line.contains(&format!("{syscall}("))
                && line.contains("INJECTED")
                && line.contains("EIO")
        })
        .ok_or_else(|| format!("strace did not inject EIO into {syscall} #{expected_ordinal}"))?;
    if injected <= marker {
        return Err(
            format!("injected {syscall} syscall occurred before the finalize marker").into()
        );
    }
    let observed_ordinal =
        lines[..=injected].iter().filter(|line| line.contains(&format!("{syscall}("))).count();
    if observed_ordinal != expected_ordinal {
        return Err(format!(
            "injected {syscall} ordinal drifted: expected {expected_ordinal}, observed {observed_ordinal}"
        )
        .into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn traced_worker(
    executable: &Path,
    db: &Path,
    trace_path: &Path,
    injection: Option<(&str, usize)>,
) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new("strace");
    command.arg("-f").arg("-qq").arg("-o").arg(trace_path);
    if let Some((syscall, ordinal)) = injection {
        command.arg("-e").arg(format!("inject={syscall}:error=EIO:when={ordinal}"));
    }
    let output = command.arg("--").arg(executable).arg("--worker").arg(db).output()?;
    Ok(output)
}

#[cfg(target_os = "linux")]
fn verify_after_fault(path: &Path) -> Result<&'static str, Box<dyn Error>> {
    let backing = RedbSpillBacking::create(path)?;
    let record = DurableRecordId::new(0);
    let unrelated = DurableRecordId::new(1);
    let state = backing.resolve_record(record)?;
    assert!(matches!(backing.resolve_record(unrelated)?, DurableRecordResolution::Present { .. }));

    let outcome = match state {
        DurableRecordResolution::Present { inventory_after: 2 } => {
            assert_eq!(backing.active_restore()?, Some(record));
            let recovery = backing
                .recover_active_restore()?
                .ok_or("Present victim lost its recovery claim")?;
            assert_eq!(recovery.candidate.sequence, 0);
            assert_eq!(recovery.candidate.bytes(), payload(0), "recovery payload changed");
            assert_eq!(recovery.phase, DurableRestorePhase::Committed);
            backing.finalize_restore(&recovery.candidate.claim)?;
            assert_eq!(backing.resolve_record(record)?, DurableRecordResolution::Finalized);
            "Present->recovered->Finalized"
        }
        DurableRecordResolution::Finalized => {
            assert_eq!(backing.active_restore()?, None);
            assert_eq!(backing.retained_count()?, 1, "only the unrelated record remains");
            assert!(backing.recover_active_restore()?.is_none());
            "Finalized"
        }
        other => return Err(format!("invalid post-fault victim state: {other:?}").into()),
    };

    let unrelated_candidate = backing.prepare_restore()?.ok_or("unrelated record disappeared")?;
    assert_eq!(unrelated_candidate.sequence, 1, "finalized victim must not be offered again");
    assert_eq!(unrelated_candidate.bytes(), payload(1), "unrelated payload changed");
    backing
        .commit_restore(&unrelated_candidate.claim)
        .map_err(|error| format!("unrelated restore commit failed: {error:?}"))?;
    backing.finalize_restore(&unrelated_candidate.claim)?;
    assert_eq!(backing.resolve_record(record)?, DurableRecordResolution::Finalized);
    assert_eq!(backing.resolve_record(unrelated)?, DurableRecordResolution::Finalized);
    assert!(backing.commit_spill(record, &payload(0)).is_err());
    assert!(backing.commit_spill(record, &payload(99)).is_err());
    assert_eq!(backing.retained_count()?, 0, "all records drain exactly once");
    assert_eq!(backing.active_restore()?, None, "no claim remains active");
    Ok(outcome)
}

#[cfg(target_os = "linux")]
fn run_scenario(root: &Path, executable: &Path, syscall: &str) -> Result<String, Box<dyn Error>> {
    let baseline_db = root.join(format!("baseline-{syscall}.redb"));
    seed(&baseline_db)?;
    let baseline_trace = root.join(format!("baseline-{syscall}.strace"));
    let baseline = traced_worker(executable, &baseline_db, &baseline_trace, None)?;
    fs::write(root.join(format!("baseline-{syscall}.stdout")), &baseline.stdout)?;
    fs::write(root.join(format!("baseline-{syscall}.stderr")), &baseline.stderr)?;
    if !baseline.status.success() {
        return Err(format!(
            "baseline finalize for {syscall} failed ({}): stdout={} stderr={}",
            baseline.status,
            String::from_utf8_lossy(&baseline.stdout),
            String::from_utf8_lossy(&baseline.stderr)
        )
        .into());
    }
    if !String::from_utf8_lossy(&baseline.stdout).contains("L2_FINALIZE_BEGIN") {
        return Err(format!(
            "baseline child did not reach the finalize hook for {syscall}: stdout={} stderr={}",
            String::from_utf8_lossy(&baseline.stdout),
            String::from_utf8_lossy(&baseline.stderr)
        )
        .into());
    }
    if !String::from_utf8_lossy(&baseline.stdout).contains("L2_FINALIZE_OK") {
        return Err(format!(
            "baseline child reached finalize hook but did not complete it for {syscall}: stdout={} stderr={}",
            String::from_utf8_lossy(&baseline.stdout),
            String::from_utf8_lossy(&baseline.stderr)
        )
        .into());
    }
    if fs::metadata(&baseline_trace)?.len() == 0 {
        return Err(
            "strace produced an empty baseline trace; verify ptrace permissions for this Linux environment"
                .into(),
        );
    }
    let baseline_text = fs::read_to_string(&baseline_trace)?;
    let ordinal = syscall_ordinal_before_marker(&baseline_text, syscall)?;
    let baseline_backing = RedbSpillBacking::create(&baseline_db)?;
    assert_eq!(
        baseline_backing.resolve_record(DurableRecordId::new(0))?,
        DurableRecordResolution::Finalized,
        "uninjected baseline must finalize the victim"
    );

    let fault_db = root.join(format!("fault-{syscall}.redb"));
    seed(&fault_db)?;
    let fault_trace = root.join(format!("fault-{syscall}.strace"));
    let fault = traced_worker(executable, &fault_db, &fault_trace, Some((syscall, ordinal)))?;
    fs::write(root.join(format!("fault-{syscall}.stdout")), &fault.stdout)?;
    fs::write(root.join(format!("fault-{syscall}.stderr")), &fault.stderr)?;
    let fault_text = fs::read_to_string(&fault_trace)?;
    verify_injection_after_marker(&fault_text, syscall, ordinal)?;
    if !String::from_utf8_lossy(&fault.stdout).contains("L2_FINALIZE_BEGIN") {
        return Err(format!(
            "faulted child failed before the finalize boundary: {}",
            String::from_utf8_lossy(&fault.stderr)
        )
        .into());
    }
    if fault.status.success() {
        return Err(format!(
            "injected {syscall} EIO did not propagate as a finalize error (stdout={} stderr={})",
            String::from_utf8_lossy(&fault.stdout),
            String::from_utf8_lossy(&fault.stderr)
        )
        .into());
    }
    let outcome = verify_after_fault(&fault_db)?;
    let receipt = format!(
        "PASS syscall={syscall} injected_ordinal={ordinal} worker_exit={} oracle={outcome}",
        fault.status
    );
    println!("{receipt}");
    Ok(receipt)
}

#[cfg(target_os = "linux")]
fn copy_artifacts(from: &Path, to: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(from)? {
        let source = entry?.path();
        if source.is_file() {
            let name = source.file_name().ok_or("artifact has no filename")?;
            fs::copy(&source, to.join(name))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn artifact_hashes(root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut artifacts: Vec<_> = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    artifacts.sort();
    let mut hashes = Vec::new();
    for path in artifacts {
        if path.file_name().is_some_and(|name| name == "receipt.txt") {
            continue;
        }
        let hash = Command::new("sha256sum").arg(&path).output()?;
        if !hash.status.success() {
            return Err(format!("sha256sum failed for {}", path.display()).into());
        }
        hashes.push(String::from_utf8_lossy(&hash.stdout).trim().to_string());
    }
    Ok(hashes)
}

#[cfg(target_os = "linux")]
struct WorkRoot {
    path: PathBuf,
    active: bool,
}

#[cfg(target_os = "linux")]
impl WorkRoot {
    fn cleanup(&mut self) -> Result<(), std::io::Error> {
        fs::remove_dir_all(&self.path)?;
        self.active = false;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for WorkRoot {
    fn drop(&mut self) {
        // Artifacts are copied to the caller-owned artifact directory first.
        // Best-effort fallback avoids leaking duplicate Redb/trace files if a
        // later receipt step returns early; normal cleanup reports failures.
        if self.active {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(target_os = "linux")]
fn effective_uid() -> Result<u32, Box<dyn Error>> {
    let status = fs::read_to_string("/proc/self/status")?;
    let uid_line = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .ok_or("/proc/self/status has no Uid field")?;
    let effective_uid =
        uid_line.split_whitespace().nth(2).ok_or("Uid field has no effective UID")?;
    effective_uid.parse::<u32>().map_err(|error| Box::new(error) as Box<dyn Error>)
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn Error>> {
    let effective_uid = effective_uid()?;
    if effective_uid != 0 {
        return Err(
            format!("L2 syscall injection requires effective UID 0; got {effective_uid}").into()
        );
    }
    if std::env::args().nth(1).as_deref() == Some("--worker") {
        let db = std::env::args().nth(2).ok_or("missing worker database path")?;
        return worker(Path::new(&db));
    }
    let strace = Command::new("strace").arg("--version").output()?;
    if !strace.status.success() {
        return Err("strace --version failed".into());
    }
    let executable = std::env::current_exe()?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let artifact_root = match std::env::var_os("QMP_DURABILITY_L2_ARTIFACT_ROOT") {
        Some(path) => PathBuf::from(path),
        None => {
            std::env::temp_dir().join(format!("qmp-durability-l2-{}-{nonce}", std::process::id()))
        }
    };
    fs::create_dir(&artifact_root)?;
    let work_root =
        std::env::temp_dir().join(format!("qmp-durability-l2-work-{}-{nonce}", std::process::id()));
    fs::create_dir(&work_root)?;
    let mut work_root = WorkRoot { path: work_root, active: true };
    println!("L2_ARTIFACT_DIR={}", artifact_root.display());
    let mut scenarios = Vec::new();
    let run_result = (|| -> Result<(), Box<dyn Error>> {
        scenarios.push(run_scenario(&work_root.path, &executable, "pwrite64")?);
        scenarios.push(run_scenario(&work_root.path, &executable, "fdatasync")?);
        Ok(())
    })();
    let fs_type = Command::new("stat").args(["-f", "-c", "%T"]).arg(&work_root.path).output()?;
    if !fs_type.status.success() {
        return Err("could not identify the L2 database filesystem".into());
    }
    let fs_type = String::from_utf8_lossy(&fs_type.stdout).trim().to_string();
    let device = Command::new("stat").args(["-c", "%D"]).arg(&work_root.path).output()?;
    if !device.status.success() {
        return Err("could not identify the L2 database filesystem device".into());
    }
    let device = String::from_utf8_lossy(&device.stdout).trim().to_string();
    copy_artifacts(&work_root.path, &artifact_root)?;
    work_root.cleanup()?;
    let strace_version =
        String::from_utf8_lossy(&strace.stdout).lines().next().unwrap_or("unknown").to_string();
    let kernel = fs::read_to_string("/proc/version")
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string();
    let architecture = Command::new("uname").arg("-m").output()?;
    if !architecture.status.success() {
        return Err("could not identify the L2 runtime architecture".into());
    }
    let architecture = String::from_utf8_lossy(&architecture.stdout).trim().to_string();
    let source_head = std::env::var("QMP_SOURCE_HEAD").unwrap_or_else(|_| "unrecorded".to_string());
    let source_digest =
        std::env::var("QMP_SOURCE_DIGEST").unwrap_or_else(|_| "unrecorded".to_string());
    let receipt_status = if let Err(error) = run_result.as_ref() {
        format!("attempts={}\npassed=0\nerror={error}\n", scenarios.len() + 1)
    } else {
        format!("attempts={}\npassed={}\n", scenarios.len(), scenarios.len())
    };
    let hashes = artifact_hashes(&artifact_root)?.join("\n");
    let receipt = format!(
        "level=L2\nsource_head={source_head}\nsource_digest={source_digest}\neffective_uid={effective_uid}\narchitecture={architecture}\nkernel={kernel}\nfilesystem={fs_type}\nfilesystem_device_id_hex={device}\nstrace={strace_version}\n{receipt_status}power_loss_proven=false\n{}\nartifact_sha256:\n{hashes}\n",
        scenarios.join("\n")
    );
    fs::write(artifact_root.join("receipt.txt"), &receipt)?;
    if let Err(error) = run_result {
        return Err(error);
    }
    println!(
        "L2_RESULT attempts=2 passed=2 source_head={source_head} source_digest={source_digest} effective_uid={effective_uid} architecture={architecture} kernel={kernel} filesystem={fs_type} strace={strace_version} receipt={}",
        artifact_root.join("receipt.txt").display()
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("UNAVAILABLE: L2 syscall injection requires Linux; no fallback is permitted");
    std::process::exit(77);
}
