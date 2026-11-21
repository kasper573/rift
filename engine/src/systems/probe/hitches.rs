//! What a slow frame waited on: every system's run timed through the span bevy opens around it,
//! and every other span the engine and bevy open (a render graph node, the frame's presentation,
//! a pipeline's compilation), so the probe can name what ran longest before a slow frame.

use std::cell::RefCell;
use std::collections::HashMap;
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

/// The layer for [`bevy::log::LogPlugin::custom_layer`] that times every system run into the
/// app's [`SystemTimes`], where [`time_systems`] asked that of the app, and keeps every span
/// closed where it didn't: a span opened costs the frame, and nothing else reads them.
pub fn timing_layer(app: &mut App) -> Option<BoxedLayer> {
    let times = app.world().get_resource::<SystemTimes>().cloned();
    Some(Box::new(SystemTimer { times }))
}

/// Has `app` time its systems, for a probe to read. Asked before the app's log plugin is added,
/// and so before any of its systems is: a system's span is opened or left closed for good when
/// the system is made.
pub fn time_systems(app: &mut App) {
    app.init_resource::<SystemTimes>();
}

/// How long each system and every other span ran since the times were last taken, of its runs of
/// [`NOTED`] or longer.
#[derive(Resource, Clone, Default)]
pub struct SystemTimes(Arc<Mutex<HashMap<String, Duration>>>);

impl SystemTimes {
    /// The `count` systems that ran longest since the last call, longest first; the rest are
    /// forgotten with them.
    pub fn take_longest(&self, count: usize) -> Vec<ProbeSystem> {
        let ran = std::mem::take(&mut *self.0.lock().expect("the system times lock"));
        let mut ran: Vec<(String, Duration)> = ran.into_iter().collect();
        ran.sort_unstable_by_key(|(_, lasted)| std::cmp::Reverse(*lasted));
        ran.into_iter()
            .take(count)
            .map(|(name, lasted)| ProbeSystem {
                name,
                ms: Millis(lasted.as_secs_f32() * 1000.0),
            })
            .collect()
    }

    fn ran(&self, name: &str, lasted: Duration) {
        let mut ran = self.0.lock().expect("the system times lock");
        match ran.get_mut(name) {
            Some(total) => *total += lasted,
            None => {
                ran.insert(name.to_owned(), lasted);
            }
        }
    }
}

/// Times spans into `times`, or keeps them all closed when it has none.
struct SystemTimer {
    times: Option<SystemTimes>,
}

/// The shortest run of a span worth keeping: shorter ones never make a frame slow, and passing
/// them by keeps the timing off the frames it times.
const NOTED: Duration = Duration::from_micros(250);

/// The name a span carries in its `name` field, as a system's and a schedule's do; any other span
/// is named by its span.
struct SpanNamed(String);

thread_local! {
    /// The spans this thread is in, each with when it entered it: a span's every run costs the
    /// frame it is timed in, so entering and leaving one touch nothing shared.
    static RUNNING: RefCell<Vec<(Id, Instant)>> = const { RefCell::new(Vec::new()) };
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SystemTimer {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.is_span() && self.times.is_none() {
            Interest::never()
        } else {
            Interest::always()
        }
    }

    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        if attributes.metadata().fields().field("name").is_none() {
            return;
        }
        let mut name = NameField(None);
        attributes.record(&mut name);
        if let Some(name) = name.0
            && let Some(span) = context.span(id)
        {
            span.extensions_mut().insert(SpanNamed(name));
        }
    }

    fn on_enter(&self, id: &Id, _: Context<'_, S>) {
        RUNNING.with_borrow_mut(|running| running.push((id.clone(), Instant::now())));
    }

    fn on_exit(&self, id: &Id, context: Context<'_, S>) {
        let began = RUNNING.with_borrow_mut(|running| {
            let entered = running.iter().rposition(|(running, _)| running == id)?;
            Some(running.remove(entered).1)
        });
        if let Some(times) = &self.times
            && let Some(began) = began
            && let lasted = began.elapsed()
            && lasted >= NOTED
            && let Some(span) = context.span(id)
        {
            match span.extensions().get::<SpanNamed>() {
                Some(named) => times.ran(&named.0, lasted),
                None => times.ran(span.name(), lasted),
            }
        }
    }
}

/// Reads the name a span carries in its `name` field, as a system's does.
struct NameField(Option<String>);

impl Visit for NameField {
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
