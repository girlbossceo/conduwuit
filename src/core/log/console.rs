use tracing::{
	field::{Field, Visit},
	Event, Level, Subscriber,
};
use tracing_subscriber::{
	field::RecordFields,
	fmt,
	fmt::{
		format::{Compact, DefaultVisitor, Format, Full, Pretty, Writer},
		FmtContext, FormatEvent, FormatFields,
	},
	registry::LookupSpan,
};

use crate::{Config, Result};

pub struct ConsoleFormat {
	_compact: Format<Compact>,
	full: Format<Full>,
	pretty: Format<Pretty>,
}

struct ConsoleVisitor<'a> {
	visitor: DefaultVisitor<'a>,
}

impl ConsoleFormat {
	#[must_use]
	pub fn new(config: &Config) -> Self {
		Self {
			_compact: fmt::format().compact(),

			full: Format::<Full>::default()
				.with_thread_ids(config.log_thread_ids)
				.with_ansi(config.log_colors),

			pretty: fmt::format()
				.pretty()
				.with_ansi(config.log_colors)
				.with_thread_names(true)
				.with_thread_ids(true)
				.with_target(true)
				.with_file(true)
				.with_line_number(true)
				.with_source_location(true),
		}
	}
}

impl<S, N> FormatEvent<S, N> for ConsoleFormat
where
	S: Subscriber + for<'a> LookupSpan<'a>,
	N: for<'a> FormatFields<'a> + 'static,
{
	fn format_event(
		&self,
		ctx: &FmtContext<'_, S, N>,
		writer: Writer<'_>,
		event: &Event<'_>,
	) -> Result<(), std::fmt::Error> {
		let is_debug =
			cfg!(debug_assertions) && event.fields().any(|field| field.name() == "_debug");

		match *event.metadata().level() {
			| Level::ERROR if !is_debug => self.pretty.format_event(ctx, writer, event),
			| _ => self.full.format_event(ctx, writer, event),
		}
	}
}

impl<'writer> FormatFields<'writer> for ConsoleFormat {
	fn format_fields<R>(&self, writer: Writer<'writer>, fields: R) -> Result<(), std::fmt::Error>
	where
		R: RecordFields,
	{
		let mut visitor = ConsoleVisitor {
			visitor: DefaultVisitor::<'_>::new(writer, true),
		};

		fields.record(&mut visitor);

		Ok(())
	}
}

impl Visit for ConsoleVisitor<'_> {
	fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
		if field.name().starts_with('_') {
			return;
		}

		self.visitor.record_debug(field, value);
	}
}
