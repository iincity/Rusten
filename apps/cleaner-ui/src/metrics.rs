use std::time::Duration;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiLatency {
    samples_us: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Percentiles {
    pub count: usize,
    pub p50_us: Option<u64>,
    pub p95_us: Option<u64>,
    pub p99_us: Option<u64>,
}

impl UiLatency {
    const CAPACITY: usize = 256;

    pub fn record(&mut self, elapsed: Duration) {
        if self.samples_us.len() == Self::CAPACITY {
            self.samples_us.remove(0);
        }
        self.samples_us
            .push(elapsed.as_micros().min(u64::MAX as u128) as u64);
    }

    pub fn percentiles(&self) -> Percentiles {
        let mut samples = self.samples_us.clone();
        samples.sort_unstable();
        Percentiles {
            count: samples.len(),
            p50_us: percentile(&samples, 50),
            p95_us: percentile(&samples, 95),
            p99_us: percentile(&samples, 99),
        }
    }
}

fn percentile(samples: &[u64], percentage: usize) -> Option<u64> {
    let last = samples.len().checked_sub(1)?;
    let index = (last * percentage).div_ceil(100);
    samples.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::UiLatency;
    use std::time::Duration;

    #[test]
    fn keeps_bounded_ui_latency_percentiles() {
        let mut latency = UiLatency::default();
        for value in 1..=300 {
            latency.record(Duration::from_micros(value));
        }
        let percentiles = latency.percentiles();
        assert_eq!(percentiles.count, 256);
        assert_eq!(percentiles.p50_us, Some(173));
        assert_eq!(percentiles.p95_us, Some(288));
        assert_eq!(percentiles.p99_us, Some(298));
    }
}
