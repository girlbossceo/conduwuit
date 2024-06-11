use std::sync::{Arc, Mutex};

use super::{super::fmt, Closure};

pub fn to_html<S>(out: &Arc<Mutex<S>>) -> Box<Closure>
where
	S: std::fmt::Write + Send + 'static,
{
	let out = out.clone();
	Box::new(move |data| {
		fmt::html(
			&mut *out.lock().expect("locked"),
			&data.level(),
			data.span_name(),
			data.message(),
		)
		.expect("log line appended");
	})
}
