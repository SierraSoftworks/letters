use std::time::Duration;

use letters::launch::{BUDGET, Bench, FIRST_FRAME, Launch};

#[test]
fn a_report_lists_each_stage_and_the_verdict() {
    let mut launch = Launch::start();
    launch.mark("document read");
    launch.mark(FIRST_FRAME);
    let report = launch.report();
    let stages: Vec<usize> = ["document read", FIRST_FRAME, "budget"].iter().filter_map(|s| report.find(s)).collect();
    assert_eq!(stages.len(), 3, "{report}");
    assert!(stages.is_sorted(), "stages are listed in the order they finished:\n{report}");
    assert!(launch.total() <= BUDGET && report.contains("within"), "{report}");
}

#[test]
fn a_bench_is_judged_on_its_median() {
    let ms = Duration::from_millis;
    let cases = [(vec![ms(90), ms(100), ms(400)], true), (vec![ms(90), ms(160), ms(170)], false), (vec![BUDGET], true)];
    for (runs, within) in cases {
        let bench = Bench { runs };
        assert_eq!(bench.within_budget(), within, "{bench:?}");
        assert!(bench.report().contains(if within { "within" } else { "over" }));
    }
}

/// Stand-ins for the app: scripts that do, or do not, report a first frame.
#[cfg(unix)]
#[test]
fn a_bench_times_each_launch_and_fails_when_one_paints_nothing() {
    use std::os::unix::fs::PermissionsExt;

    let folder = std::env::temp_dir().join(format!("letters-launch-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let script = |name: &str, body: &str| {
        let path = folder.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let paints = script("paints", "echo 'first frame'; echo 'the rest of the report'");
    let silent = script("silent", "exit 0");
    let crashes = script("crashes", "echo 'first frame'; exit 3");

    let bench = letters::launch::bench(&paints, None, true, 3).unwrap();
    assert_eq!(bench.runs.len(), 3);
    assert!(bench.runs.is_sorted());
    assert!(letters::launch::bench(&silent, None, false, 1).is_err());
    assert!(letters::launch::bench(&crashes, None, false, 1).is_err());
    std::fs::remove_dir_all(&folder).unwrap();
}
