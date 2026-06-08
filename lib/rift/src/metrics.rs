use std::time::{SystemTime, UNIX_EPOCH};

pub struct Metrics {
    pub ticks: u64,
    pub tick_duration: Histogram,
    pub tick_interval: Histogram,
    pub connected: u64,
    pub opened: u64,
    pub closed: Vec<(&'static str, u64)>, // close reason -> count
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub entities: u64,
    started: f64, // unix seconds at construction
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            ticks: 0,
            tick_duration: Histogram::new(SECONDS_BUCKETS),
            tick_interval: Histogram::new(SECONDS_BUCKETS),
            connected: 0,
            opened: 0,
            closed: Vec::new(),
            bytes_sent: 0,
            bytes_received: 0,
            packets_sent: 0,
            packets_received: 0,
            entities: 0,
            started: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_secs_f64())
                .unwrap_or(0.0),
        }
    }
}

impl Metrics {
    pub fn close(&mut self, reason: &'static str) {
        match self.closed.iter_mut().find(|(key, _)| *key == reason) {
            Some((_, count)) => *count += 1,
            None => self.closed.push((reason, 1)),
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(4096);
        counter(&mut out, "rift_ticks_total", "Game ticks run", self.ticks);
        self.tick_duration.render(
            &mut out,
            "rift_tick_duration_seconds",
            "Time spent per tick",
        );
        self.tick_interval
            .render(&mut out, "rift_tick_interval_seconds", "Time between ticks");
        gauge(
            &mut out,
            "rift_clients_connected",
            "Clients currently connected",
            self.connected as f64,
        );
        counter(
            &mut out,
            "rift_client_connections_opened_total",
            "Connections accepted past the handshake",
            self.opened,
        );
        header(
            &mut out,
            "rift_client_connections_closed_total",
            "Connections closed, by reason",
            "counter",
        );
        for &(code, count) in &self.closed {
            out.push_str(&format!(
                "rift_client_connections_closed_total{{code=\"{code}\"}} {count}\n"
            ));
        }
        counter(
            &mut out,
            "rift_bytes_sent_total",
            "Bytes written to clients",
            self.bytes_sent,
        );
        counter(
            &mut out,
            "rift_bytes_received_total",
            "Bytes read from clients",
            self.bytes_received,
        );
        counter(
            &mut out,
            "rift_packets_sent_total",
            "Packets sent to clients",
            self.packets_sent,
        );
        counter(
            &mut out,
            "rift_packets_received_total",
            "Packets received from clients",
            self.packets_received,
        );
        gauge(
            &mut out,
            "rift_entities",
            "Live entities across all shards",
            self.entities as f64,
        );
        gauge(
            &mut out,
            "process_start_time_seconds",
            "Unix time the process started",
            self.started,
        );
        if let Some(rss) = resident_memory_bytes() {
            gauge(
                &mut out,
                "process_resident_memory_bytes",
                "Resident memory",
                rss,
            );
        }
        if let Some(cpu) = cpu_seconds_total() {
            counter_f64(
                &mut out,
                "process_cpu_seconds_total",
                "CPU time consumed",
                cpu,
            );
        }
        out
    }
}

pub struct Histogram {
    bounds: &'static [f64],
    counts: Vec<u64>,
    sum: f64,
    count: u64,
}

const SECONDS_BUCKETS: &[f64] = &[
    0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
];

impl Histogram {
    fn new(bounds: &'static [f64]) -> Self {
        Self {
            bounds,
            counts: vec![0; bounds.len()],
            sum: 0.0,
            count: 0,
        }
    }

    pub fn observe(&mut self, value: f64) {
        for (index, &bound) in self.bounds.iter().enumerate() {
            if value <= bound {
                self.counts[index] += 1;
            }
        }
        self.sum += value;
        self.count += 1;
    }

    fn render(&self, out: &mut String, name: &str, help: &str) {
        header(out, name, help, "histogram");
        for (index, &bound) in self.bounds.iter().enumerate() {
            out.push_str(&format!(
                "{name}_bucket{{le=\"{bound}\"}} {}\n",
                self.counts[index]
            ));
        }
        out.push_str(&format!("{name}_bucket{{le=\"+Inf\"}} {}\n", self.count));
        out.push_str(&format!("{name}_sum {}\n", self.sum));
        out.push_str(&format!("{name}_count {}\n", self.count));
    }
}

fn header(out: &mut String, name: &str, help: &str, kind: &str) {
    out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n"));
}

fn counter(out: &mut String, name: &str, help: &str, value: u64) {
    header(out, name, help, "counter");
    out.push_str(&format!("{name} {value}\n"));
}

fn counter_f64(out: &mut String, name: &str, help: &str, value: f64) {
    header(out, name, help, "counter");
    out.push_str(&format!("{name} {value}\n"));
}

fn gauge(out: &mut String, name: &str, help: &str, value: f64) {
    header(out, name, help, "gauge");
    out.push_str(&format!("{name} {value}\n"));
}

fn resident_memory_bytes() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kilobytes: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kilobytes * 1024.0)
}

fn cpu_seconds_total() -> Option<f64> {
    // schedstat field 1 is on-cpu time in nanoseconds; exact without knowing the tick rate.
    let schedstat = std::fs::read_to_string("/proc/self/schedstat").ok()?;
    let nanoseconds: f64 = schedstat.split_whitespace().next()?.parse().ok()?;
    Some(nanoseconds / 1e9)
}
