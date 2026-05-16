use std::process::Command;
use std::sync::atomic::{AtomicU16, Ordering};

static NEXT_PORT: AtomicU16 = AtomicU16::new(15000);

fn run_raptor(workers: usize, max_runs: u64, contract_name: &str) -> String {
    let broker_port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
    let bin = env!("CARGO_BIN_EXE_raptor");
    let output = Command::new(bin)
        .arg("fuzz")
        .arg(format!("--workers={}", workers))
        .arg(format!("--max-runs={}", max_runs))
        .arg(format!("--broker-port={}", broker_port))
        .arg("-p")
        .arg("fixtures/basic-target")
        .arg(format!("test/{}", contract_name))
        .output()
        .expect("failed to spawn raptor");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // For single-worker runs the binary must exit cleanly.
    if workers == 1 && !output.status.success() {
        panic!(
            "raptor exited with {}\nstdout:\n{}\nstderr:\n{}",
            output.status, stdout, stderr
        );
    }

    stdout.into_owned()
}

/// Extract the last run count from the output. The broker prints the
/// aggregated total once; child stdout is redirected so there are no
/// duplicate lines.
fn extract_runs(output: &str) -> u64 {
    for line in output.lines().rev() {
        if let Some(rest) = line.strip_prefix("Fuzzing completed: ") {
            if let Some(num) = rest.strip_suffix(" runs") {
                return num.parse().expect("invalid run count");
            }
        }
    }
    panic!("could not find run count in output:\n{}", output);
}

#[test]
fn max_runs_with_one_worker() {
    let out = run_raptor(1, 1000, "ImpossibleBug.sol");
    let runs = extract_runs(&out);
    assert_eq!(runs, 1000, "single worker should run all 1000 runs");
}

#[test]
fn max_runs_with_three_workers() {
    let out = run_raptor(3, 1000, "ImpossibleBug.sol");
    let runs = extract_runs(&out);
    assert_eq!(runs, 1000, "total runs across 3 workers should be 1000");
}

#[test]
fn max_runs_with_four_workers() {
    let out = run_raptor(4, 1000, "ImpossibleBug.sol");
    let runs = extract_runs(&out);
    assert_eq!(runs, 1000, "total runs across 4 workers should be 1000");
}
