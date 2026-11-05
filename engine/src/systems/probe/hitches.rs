//! What a slow frame waited on: every system's run timed through the span bevy opens around it,
//! so the probe can name the systems that ran longest before a slow frame.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::log::BoxedLayer;
use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::span::{Attributes, Id};
use bevy::log::tracing::subscriber::Interest;
use bevy::log::tracing::{Metadata, Subscriber};
use bevy::log::tracing_subscriber::layer::{Context, Layer};
use bevy::log::tracing_subscriber::registry::LookupSpan;
use bevy::prelude::*;

use super::frames::ProbeSystem;
use crate::core::units::Millis;

/// The layer for [`bevy::log::LogPlugin::custom_layer`] that times every system run, kept in the
/// app's [`SystemTimes`] once a probe listens to them.
pub fn timing_layer(app: &mut App) -> Option<BoxedLayer> {
    let times = SystemTimes::default();
    app.insert_resource(times.clone());
    Some(Box::new(SystemTimer { times }))
}

/// How long each system ran since the times were last taken, of its runs of [`NOTED`] or longer,
/// timed only while something listens.
#[derive(Resource, Clone, Default)]
pub struct SystemTimes(Arc<Timed>);

impl SystemTimes {
    pub fn listen(&self) {
        self.0.listening.store(true, Ordering::Relaxed);
    }

    /// The `count` systems that ran longest since the last call, longest first; the rest are
    /// forgotten with them.
    pub fn take_longest(&self, count: usize) -> Vec<ProbeSystem> {
        let ran = std::mem::take(&mut *self.0.ran.lock().expect("the system times lock"));
        let mut ran: Vec<(Arc<str>, Duration)> = ran.into_iter().collect();
        ran.sort_unstable_by_key(|(_, lasted)| std::cmp::Reverse(*lasted));
        ran.into_iter()
            .take(count)
            .map(|(name, lasted)| ProbeSystem {
                name: name.to_string(),
                ms: Millis(lasted.as_secs_f32() * 1000.0),
            })
            .collect()
    }

    fn listening(&self) -> bool {
        self.0.listening.load(Ordering::Relaxed)
    }

    fn ran(&self, name: &Arc<str>, lasted: Duration) {
        let mut ran = self.0.ran.lock().expect("the system times lock");
        *ran.entry(name.clone()).or_default() += lasted;
    }
}

#[derive(Default)]
struct Timed {
    listening: AtomicBool,
    ran: Mutex<HashMap<Arc<str>, Duration>>,
}

struct SystemTimer {
    times: SystemTimes,
}

/// The shortest run of a system worth keeping: shorter ones never make a frame slow, and passing
/// them by keeps the timing off the frames it times.
const NOTED: Duration = Duration::from_micros(250);

/// The span bevy opens around every run of a system, named by the system it runs.
const SYSTEM_SPAN: &str = "system";

/// A system's span: the system's name, and when its current run began.
struct SpanRun {
    name: Arc<str>,
    began: Option<Instant>,
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SystemTimer {
    /// Asks of every span each time whether it is wanted, as that changes once a probe listens.
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.is_span() {
            Interest::sometimes()
        } else {
            Interest::always()
        }
    }

    /// Opens only systems' spans, and those only while a probe listens: every span opened costs
    /// the frame, and nothing else reads them.
    fn enabled(&self, metadata: &Metadata<'_>, _: Context<'_, S>) -> bool {
        !metadata.is_span() || (metadata.name() == SYSTEM_SPAN && self.times.listening())
    }

    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        if attributes.metadata().name() != SYSTEM_SPAN {
            return;
        }
        let mut name = SpanName(None);
        attributes.record(&mut name);
        if let (Some(name), Some(span)) = (name.0, context.span(id)) {
            span.extensions_mut().insert(SpanRun {
                name: name.into(),
                began: None,
            });
        }
    }

    fn on_enter(&self, id: &Id, context: Context<'_, S>) {
        if !self.times.listening() {
            return;
        }
        if let Some(span) = context.span(id)
            && let Some(run) = span.extensions_mut().get_mut::<SpanRun>()
        {
            run.began = Some(Instant::now());
        }
    }

    fn on_exit(&self, id: &Id, context: Context<'_, S>) {
        if !self.times.listening() {
            return;
        }
        if let Some(span) = context.span(id)
            && let Some(run) = span.extensions_mut().get_mut::<SpanRun>()
            && let Some(began) = run.began.take()
            && let lasted = began.elapsed()
            && lasted >= NOTED
        {
            self.times.ran(&run.name, lasted);
        }
    }
}

/// Reads the name a system's span carries.
struct SpanName(Option<String>);

impl Visit for SpanName {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "name" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "name" {
            self.0 = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }
}
