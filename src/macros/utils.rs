use syn::{parse_str, Expr, Generics, Lit, Meta};

use crate::Result;

pub(crate) fn is_cargo_build() -> bool {
	std::env::args()
		.find(|flag| flag.starts_with("--emit"))
		.as_ref()
		.and_then(|flag| flag.split_once('='))
		.map(|val| val.1.split(','))
		.and_then(|mut vals| vals.find(|elem| *elem == "link"))
		.is_some()
}

pub(crate) fn get_named_generics(args: &[Meta], name: &str) -> Result<Generics> {
	const DEFAULT: &str = "<>";

	parse_str::<Generics>(&get_named_string(args, name).unwrap_or_else(|| DEFAULT.to_owned()))
}

pub(crate) fn get_named_string(args: &[Meta], name: &str) -> Option<String> {
	args.iter().find_map(|arg| {
		let value = arg.require_name_value().ok()?;
		let Expr::Lit(ref lit) = value.value else {
			return None;
		};
		let Lit::Str(ref str) = lit.lit else {
			return None;
		};
		value.path.is_ident(name).then_some(str.value())
	})
}

#[must_use]
pub(crate) fn camel_to_snake_string(s: &str) -> String {
	let mut output = String::with_capacity(
		s.chars()
			.fold(s.len(), |a, ch| a.saturating_add(usize::from(ch.is_ascii_uppercase()))),
	);

	let mut state = false;
	s.chars().for_each(|ch| {
		let m = ch.is_ascii_uppercase();
		let s = exchange(&mut state, !m);
		if m && s {
			output.push('_');
		}
		output.push(ch.to_ascii_lowercase());
	});

	output
}

#[inline]
pub(crate) fn exchange<T>(state: &mut T, source: T) -> T { std::mem::replace(state, source) }
