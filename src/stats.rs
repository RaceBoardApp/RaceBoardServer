use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStats {
    pub recent_times: VecDeque<i64>,
    pub mean: f64,
    pub median: f64,
    pub std_dev: f64,
    pub mad: f64, // Median Absolute Deviation
    pub percentiles: Percentiles,
    pub trend: TrendAnalysis,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Percentiles {
    pub p10: i64,
    pub p25: i64,
    pub p50: i64, // median
    pub p75: i64,
    pub p90: i64,
    pub p95: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysis {
    pub direction: TrendDirection,
    pub rate: f64, // Change rate per execution
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    Improving,
    Degrading,
    Stable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtaPrediction {
    pub expected_seconds: i64,
    pub confidence: f64,
    pub lower_bound: i64,
    pub upper_bound: i64,
}

impl ExecutionStats {
    pub fn new() -> Self {
        Self {
            recent_times: VecDeque::new(),
            mean: 0.0,
            median: 0.0,
            std_dev: 0.0,
            mad: 0.0,
            percentiles: Percentiles {
                p10: 0,
                p25: 0,
                p50: 0,
                p75: 0,
                p90: 0,
                p95: 0,
            },
            trend: TrendAnalysis {
                direction: TrendDirection::Stable,
                rate: 0.0,
                confidence: 0.0,
            },
            last_updated: Utc::now(),
        }
    }

    pub fn new_with_default(default_eta: i64) -> Self {
        let mut stats = Self::new();
        stats.recent_times.push_back(default_eta);
        stats.mean = default_eta as f64;
        stats.median = default_eta as f64;
        stats.percentiles.p50 = default_eta;
        stats.percentiles.p25 = (default_eta as f64 * 0.75) as i64;
        stats.percentiles.p75 = (default_eta as f64 * 1.25) as i64;
        stats
    }

    pub fn is_anomaly(&self, duration: i64) -> bool {
        if self.mad == 0.0 || self.recent_times.is_empty() {
            return false;
        }

        let modified_z_score = 0.6745 * (duration as f64 - self.median).abs() / self.mad;
        modified_z_score > 3.5
    }

    pub fn update_with_duration(&mut self, duration: i64) {
        if self.is_anomaly(duration) {
            eprintln!(
                "Anomalous execution detected: {}s (median: {}s)",
                duration, self.median
            );
            return;
        }

        self.recent_times.push_back(duration);
        if self.recent_times.len() > 20 {
            self.recent_times.pop_front();
        }

        self.recalculate();
        self.last_updated = Utc::now();
    }

    pub fn recalculate(&mut self) {
        if self.recent_times.is_empty() {
            return;
        }

        let times = &self.recent_times;
        let count = times.len() as f64;

        // Calculate mean
        self.mean = times.iter().sum::<i64>() as f64 / count;

        // Calculate median
        let mut sorted_times: Vec<i64> = times.iter().copied().collect();
        sorted_times.sort_unstable();
        let mid = sorted_times.len() / 2;
        self.median = if sorted_times.len() % 2 == 0 {
            (sorted_times[mid - 1] + sorted_times[mid]) as f64 / 2.0
        } else {
            sorted_times[mid] as f64
        };

        // Calculate standard deviation
        let variance = times
            .iter()
            .map(|&x| {
                let diff = x as f64 - self.mean;
                diff * diff
            })
            .sum::<f64>()
            / count;
        self.std_dev = variance.sqrt();

        // Calculate MAD
        let mad_values: Vec<i64> = times
            .iter()
            .map(|&t| (t as f64 - self.median).abs() as i64)
            .collect();
        let mut sorted_mad = mad_values;
        sorted_mad.sort_unstable();
        self.mad = if sorted_mad.len() % 2 == 0 {
            (sorted_mad[mid - 1] + sorted_mad[mid]) as f64 / 2.0
        } else {
            sorted_mad[mid] as f64
        };

        // Calculate percentiles
        self.calculate_percentiles(&sorted_times);

        // Analyze trend
        self.analyze_trend();
    }

    fn calculate_percentiles(&mut self, sorted_times: &[i64]) {
        let len = sorted_times.len();
        if len == 0 {
            return;
        }

        self.percentiles.p10 = sorted_times[(len as f64 * 0.1) as usize];
        self.percentiles.p25 = sorted_times[(len as f64 * 0.25) as usize];
        self.percentiles.p50 = sorted_times[len / 2];
        self.percentiles.p75 = sorted_times[(len as f64 * 0.75) as usize];
        self.percentiles.p90 = sorted_times[(len as f64 * 0.9) as usize];
        self.percentiles.p95 = sorted_times[((len as f64 * 0.95) as usize).min(len - 1)];
    }

    fn analyze_trend(&mut self) {
        if self.recent_times.len() < 5 {
            self.trend.confidence = 0.0;
            self.trend.direction = TrendDirection::Stable;
            return;
        }

        // Simple trend analysis: compare first half with second half
        let mid = self.recent_times.len() / 2;
        let first_half_avg: f64 =
            self.recent_times.iter().take(mid).sum::<i64>() as f64 / mid as f64;
        let second_half_avg: f64 = self.recent_times.iter().skip(mid).sum::<i64>() as f64
            / (self.recent_times.len() - mid) as f64;

        let change_rate = (second_half_avg - first_half_avg).abs() / first_half_avg;
        self.trend.rate = change_rate;

        if change_rate < 0.05 {
            self.trend.direction = TrendDirection::Stable;
            self.trend.confidence = 0.8;
        } else if second_half_avg < first_half_avg {
            self.trend.direction = TrendDirection::Improving;
            self.trend.confidence = change_rate.min(0.95);
        } else {
            self.trend.direction = TrendDirection::Degrading;
            self.trend.confidence = change_rate.min(0.95);
        }
    }

    pub fn calculate_eta(&self) -> EtaPrediction {
        let base = self.median;

        let adjusted = if self.trend.confidence > 0.7 {
            match self.trend.direction {
                TrendDirection::Improving => base * (1.0 - self.trend.rate.min(0.2)),
                TrendDirection::Degrading => base * (1.0 + self.trend.rate.min(0.2)),
                TrendDirection::Stable => base,
            }
        } else {
            base
        };

        EtaPrediction {
            expected_seconds: adjusted as i64,
            confidence: self.calculate_confidence(),
            lower_bound: self.percentiles.p25,
            upper_bound: self.percentiles.p75,
        }
    }

    fn calculate_confidence(&self) -> f64 {
        if self.recent_times.is_empty() {
            return 0.0;
        }

        let sample_factor = (self.recent_times.len() as f64 / 20.0).min(1.0);
        let consistency_factor = if self.mean > 0.0 {
            1.0 / (1.0 + (self.std_dev / self.mean).min(1.0))
        } else {
            0.0
        };

        (sample_factor * 0.6 + consistency_factor * 0.4).min(0.95)
    }
}

impl Default for ExecutionStats {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anomaly_detection() {
        let mut stats = ExecutionStats::new();
        stats.update_with_duration(10);
        stats.update_with_duration(12);
        stats.update_with_duration(11);
        stats.update_with_duration(100); // Anomaly
        assert_eq!(stats.recent_times.len(), 3); // Anomaly not included
    }

    #[test]
    fn test_confidence_calculation() {
        let mut stats = ExecutionStats::new();
        assert_eq!(stats.calculate_confidence(), 0.0);

        for i in 0..20 {
            stats.update_with_duration(10 + i % 2);
        }

        let confidence = stats.calculate_confidence();
        assert!(confidence > 0.8);
        assert!(confidence <= 0.95);
    }
}
