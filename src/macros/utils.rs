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

pub(crate) fn exchange<T: Clone>(state: &mut T, source: T) -> T {
	let ret = state.clone();
	*state = source;
	ret
}
