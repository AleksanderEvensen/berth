#![allow(clippy::unwrap_used, clippy::float_cmp)]
use super::*;
use bollard::models::ContainerCpuUsage;

fn cpu(
    total: u64,
    system: u64,
    online: Option<u32>,
    per_cpu: Option<Vec<u64>>,
) -> ContainerCpuStats {
    ContainerCpuStats {
        cpu_usage: Some(ContainerCpuUsage {
            total_usage: Some(total),
            percpu_usage: per_cpu,
            ..Default::default()
        }),
        system_cpu_usage: Some(system),
        online_cpus: online,
        ..Default::default()
    }
}
#[test]
fn cpu_cli_formula_and_fallbacks() {
    let previous = cpu(100, 1000, None, None);
    let mut current = cpu(300, 1400, Some(4), Some(vec![0; 2]));
    assert_eq!(cpu_percent(&current, &previous, cpus(&current)), 200.0);
    current.online_cpus = None;
    assert_eq!(cpus(&current), 2);
    current.online_cpus = Some(0);
    assert_eq!(cpus(&current), 2);
    current.cpu_usage.as_mut().unwrap().percpu_usage = Some(vec![]);
    assert_eq!(cpus(&current), 1);
    assert_eq!(cpus(&ContainerCpuStats::default()), 1);
}
#[test]
fn empty_previous_missing_fields_and_underflow_are_zero() {
    let current = cpu(300, 1400, Some(4), None);
    for previous in [
        ContainerCpuStats::default(),
        cpu(400, 1000, None, None),
        cpu(100, 1500, None, None),
        cpu(100, 1400, None, None),
        cpu(300, 1000, None, None),
        cpu(u64::MAX, u64::MAX, None, None),
    ] {
        assert_eq!(cpu_percent(&current, &previous, 4), 0.0);
    }
    assert_eq!(cpu_percent(&ContainerCpuStats::default(), &current, 4), 0.0);
    assert_eq!(float(0), 0.0);
    assert_eq!(float(u64::from(u32::MAX)), 4_294_967_295.0);
    assert_eq!(float(1_u64 << 32), 4_294_967_296.0);
    assert_eq!(float(u64::MAX), 18_446_744_073_709_551_616.0);
}
#[test]
fn memory_v1_v2_missing_and_invalid_cache() {
    for (stats, used) in [
        (
            serde_json::json!({"total_inactive_file": 30, "inactive_file": 10}),
            70,
        ),
        (serde_json::json!({"inactive_file": 10}), 90),
        (
            serde_json::json!({"total_inactive_file": 110, "inactive_file": 10}),
            90,
        ),
        (serde_json::json!({"inactive_file": 100}), 100),
        (serde_json::json!({"inactive_file": 110}), 100),
        (serde_json::json!({}), 100),
    ] {
        let memory: ContainerMemoryStats =
            serde_json::from_value(serde_json::json!({"usage":100, "stats":stats})).unwrap();
        assert_eq!(memory_used(&memory), used);
    }
    assert_eq!(memory_used(&ContainerMemoryStats::default()), 0);
}
#[test]
fn sample_sums_network_and_block_io_saturating_and_maps_read_timestamp() {
    let value = serde_json::from_value(serde_json::json!({
        "read": "1970-01-01T00:00:01.123456789Z", "networks": {"eth0":{"rx_bytes":10,"tx_bytes":20},"eth1":{"rx_bytes":30,"tx_bytes":40}},
        "blkio_stats":{"io_service_bytes_recursive":[{"op":"Read","value":10},{"op":"read","value":20},{"op":"WRITE","value":40},{"op":"Total","value":1000},{}]},
        "memory_stats":{"usage":100,"limit":1000,"stats":{"inactive_file":30}}
    })).unwrap();
    let s = sample(&value);
    assert_eq!(
        (s.net_rx, s.net_tx, s.blk_read, s.blk_write),
        (40, 60, 30, 40)
    );
    assert_eq!((s.mem_used, s.mem_limit), (70, 1000));
    assert_eq!(s.time_nanos, 1_123_456_789);
    let missing = sample(&ContainerStatsResponse::default());
    assert_eq!(
        (
            missing.cpu_percent,
            missing.online_cpus,
            missing.mem_used,
            missing.net_rx
        ),
        (0.0, 1, 0, 0)
    );
    let overflow = serde_json::from_value(serde_json::json!({"networks":{"a":{"rx_bytes":u64::MAX},"b":{"rx_bytes":1}},"blkio_stats":{"io_service_bytes_recursive":[{"op":"read","value":u64::MAX},{"op":"Read","value":1}]}})).unwrap();
    let overflow = sample(&overflow);
    assert_eq!((overflow.net_rx, overflow.blk_read), (u64::MAX, u64::MAX));
}
