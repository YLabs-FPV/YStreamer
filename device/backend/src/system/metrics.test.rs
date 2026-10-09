use super::*;

#[test]
fn reads_processor_times() {
    let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 10 0 5 80 5 0 0 0 0 0\nintr 1 2 3\n";
    assert_eq!(
        cpu_times(stat),
        [
            ("cpu".to_string(), 150, 1000),
            ("cpu0".to_string(), 15, 100)
        ]
    );
}

#[test]
fn reads_own_ticks_past_a_name_with_spaces() {
    let stat = "123 (my prog) S 1 2 3 4 5 6 7 8 9 10 40 60 0 0";
    assert_eq!(own_ticks(stat), Some(100));
}

#[test]
fn keeps_only_real_interfaces() {
    let dev = "Inter-|   Receive |  Transmit\n face |bytes |bytes\n\
               \x20   lo: 9 0 0 0 0 0 0 0 9 0 0 0 0 0 0 0\n\
               \x20 eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n\
               docker0: 5 0 0 0 0 0 0 0 5 0 0 0 0 0 0 0\n";
    assert_eq!(interfaces(dev), [("eth0".to_string(), 1000, 2000)]);
}

#[test]
fn reads_the_wifi_signal_only_when_linked() {
    let head =
        "Inter-| sta-|   Quality        |   Discarded\n face | tus | link level noise |  nwid\n";
    let linked = format!("{head} wlan0: 0000   54.  -56.  -256        0\n");
    assert_eq!(wifi_signal(&linked), Some(-56.0));
    let idle = format!("{head} wlan0: 0000    0.    0.  -256        0\n");
    assert_eq!(wifi_signal(&idle), None);
    assert_eq!(wifi_signal(head), None);
}

#[test]
fn a_snapshot_returns_only_newer_samples_and_fills_gaps() {
    let metrics = Metrics::new();
    {
        let mut history = metrics.history.lock().unwrap();
        for (seq, values) in [
            (1, vec![("cpu", 10.0)]),
            (2, vec![("cpu", 20.0), ("wifi_dbm", -50.0)]),
            (3, vec![("cpu", 30.0)]),
        ] {
            history.push_back(Sample {
                seq,
                unix_ms: seq * 1000,
                values: values
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            });
        }
    }
    let all = metrics.snapshot(0);
    assert_eq!((all.first, all.seq), (1, 3));
    assert_eq!(all.series["wifi_dbm"], [None, Some(-50.0), None]);

    let newer = metrics.snapshot(2);
    assert_eq!(newer.t, [3000]);
    assert_eq!(newer.series["cpu"], [Some(30.0)]);
    assert!(!newer.series.contains_key("wifi_dbm"));

    // A number from before a restart gets everything
    assert_eq!(metrics.snapshot(99).t.len(), 3);
}
