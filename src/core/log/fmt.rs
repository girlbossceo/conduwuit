use std::fmt::Write;

use super::{color, Level};
use crate::Result;

pub fn html<S>(out: &mut S, level: &Level, span: &str, msg: &str) -> Result<()>
where
	S: Write,
{
	let color = color::code_tag(level);
	let level = level.as_str().to_uppercase();
	write!(
		out,
		"<font data-mx-color=\"{color}\"><code>{level:>5}</code></font> <code>{span:^12}</code> <code>{msg}</code><br>"
	)?;

	Ok(())
}
