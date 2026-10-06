use crate::StatsSample;
use bollard::models::{ContainerCpuStats, ContainerMemoryStats, ContainerStatsResponse};

// Each half converts exactly. Combining them rounds once to f64's available
// precision (as Docker CLI's float64 conversion does), without a lossy cast.
fn float(value: u64) -> f64 {
    let high = u32::try_from(value >> 32).unwrap_or_default();
    let low = u32::try_from(value & u64::from(u32::MAX)).unwrap_or_default();
    f64::from(high).mul_add(4_294_967_296.0, f64::from(low))
}
fn cpus(cpu: &ContainerCpuStats) -> u32 {
    cpu.online_cpus
        .filter(|n| *n > 0)
        .or_else(|| {
            cpu.cpu_usage
                .as_ref()?
                .percpu_usage
                .as_ref()
                .and_then(|values| u32::try_from(values.len()).ok())
                .filter(|n| *n > 0)
        })
        .unwrap_or(1)
}
fn cpu_percent(current: &ContainerCpuStats, previous: &ContainerCpuStats, online: u32) -> f64 {
    let total =
        |cpu: &ContainerCpuStats| cpu.cpu_usage.as_ref().and_then(|usage| usage.total_usage);
    let (Some(total), Some(pretotal), Some(system), Some(presystem)) = (
        total(current),
        total(previous),
        current.system_cpu_usage,
        previous.system_cpu_usage,
    ) else {
        return 0.0;
    };
    let cpu_delta = total.saturating_sub(pretotal);
    let system_delta = system.saturating_sub(presystem);
    if system_delta == 0 || cpu_delta == 0 {
        return 0.0;
    }
    float(cpu_delta) / float(system_delta) * f64::from(online) * 100.0
}
fn memory_used(memory: &ContainerMemoryStats) -> u64 {
    let usage = memory
        .usage
        .or(memory.privateworkingset)
        .unwrap_or_default();
    let Some(stats) = memory.stats.as_ref() else {
        return usage;
    };
    // CLI tries v1 first, then v2, and subtracts only when below usage.
    let cache = stats
        .get("total_inactive_file")
        .filter(|v| **v < usage)
        .or_else(|| stats.get("inactive_file").filter(|v| **v < usage));
    usage.saturating_sub(cache.copied().unwrap_or_default())
}
pub(super) fn sample(value: &ContainerStatsResponse) -> StatsSample {
    let empty_cpu = ContainerCpuStats::default();
    let empty_memory = ContainerMemoryStats::default();
    let cpu = value.cpu_stats.as_ref().unwrap_or(&empty_cpu);
    let previous = value.precpu_stats.as_ref().unwrap_or(&empty_cpu);
    let memory = value.memory_stats.as_ref().unwrap_or(&empty_memory);
    let online_cpus = cpus(cpu);
    let (net_rx, net_tx) =
        value
            .networks
            .iter()
            .flat_map(|n| n.values())
            .fold((0_u64, 0_u64), |(rx, tx), n| {
                (
                    rx.saturating_add(n.rx_bytes.unwrap_or_default()),
                    tx.saturating_add(n.tx_bytes.unwrap_or_default()),
                )
            });
    let (blk_read, blk_write) = value
        .blkio_stats
        .as_ref()
        .and_then(|b| b.io_service_bytes_recursive.as_ref())
        .into_iter()
        .flatten()
        .fold((0_u64, 0_u64), |(read, write), b| match b.op.as_deref() {
            Some(op) if op.eq_ignore_ascii_case("read") => {
                (read.saturating_add(b.value.unwrap_or_default()), write)
            }
            Some(op) if op.eq_ignore_ascii_case("write") => {
                (read, write.saturating_add(b.value.unwrap_or_default()))
            }
            _ => (read, write),
        });
    StatsSample {
        time_nanos: value
            .read
            .as_ref()
            .and_then(chrono::DateTime::timestamp_nanos_opt)
            .unwrap_or_default(),
        cpu_percent: cpu_percent(cpu, previous, online_cpus),
        online_cpus,
        mem_used: memory_used(memory),
        mem_limit: memory.limit.unwrap_or_default(),
        net_rx,
        net_tx,
        blk_read,
        blk_write,
    }
}

#[cfg(test)]
mod tests;
