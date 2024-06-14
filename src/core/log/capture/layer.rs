use std::{fmt, sync::Arc};

use tracing::field::{Field, Visit};
use tracing_core::{Event, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan};

use super::{Capture, Data, State};

pub type Value = (&'static str, String);

pub struct Layer {
	state: Arc<State>,
}

struct Visitor {
	values: Vec<Value>,
}

impl Layer {
	pub fn new(state: &Arc<State>) -> Self {
		Self {
			state: state.clone(),
		}
	}
}

impl fmt::Debug for Layer {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.debug_struct("capture::Layer").finish()
	}
}

impl<S> tracing_subscriber::Layer<S> for Layer
where
	S: Subscriber + for<'a> LookupSpan<'a>,
{
	fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
		self.state
			.active
			.read()
			.expect("shared lock")
			.iter()
			.filter(|capture| filter(self, capture, event, &ctx))
			.for_each(|capture| handle(self, capture, event, &ctx));
	}
}

fn handle<S>(layer: &Layer, capture: &Capture, event: &Event<'_>, ctx: &Context<'_, S>)
where
	S: Subscriber + for<'a> LookupSpan<'a>,
{
	let mut visitor = Visitor {
		values: Vec::new(),
	};
	event.record(&mut visitor);

	let mut closure = capture.closure.lock().expect("exclusive lock");
	closure(Data {
		layer,
		event,
		current: &ctx.current_span(),
		values: Some(&mut visitor.values),
	});
}

fn filter<S>(layer: &Layer, capture: &Capture, event: &Event<'_>, ctx: &Context<'_, S>) -> bool
where
	S: Subscriber + for<'a> LookupSpan<'a>,
{
	capture.filter.as_ref().map_or(true, |filter| {
		filter(Data {
			layer,
			event,
			current: &ctx.current_span(),
			values: None,
		})
	})
}

impl Visit for Visitor {
	fn record_debug(&mut self, f: &Field, v: &dyn fmt::Debug) { self.values.push((f.name(), format!("{v:?}"))); }

	fn record_str(&mut self, f: &Field, v: &str) { self.values.push((f.name(), v.to_owned())); }
}
