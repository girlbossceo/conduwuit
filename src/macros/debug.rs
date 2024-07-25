use std::cmp;

use proc_macro::TokenStream;
use syn::{parse_macro_input, AttributeArgs, Item};

pub(super) fn recursion_depth(args: TokenStream, item_: TokenStream) -> TokenStream {
	let item = item_.clone();
	let item = parse_macro_input!(item as Item);
	let _args = parse_macro_input!(args as AttributeArgs);

	let mut best: usize = 0;
	let mut count: usize = 0;
	// think you'd find a fancy recursive ast visitor? think again
	let tree = format!("{item:#?}");
	for line in tree.lines() {
		let trim = line.trim_start_matches(' ');
		let diff = line.len().saturating_sub(trim.len());
		let level = diff / 4;
		best = cmp::max(level, best);
		count = count.saturating_add(1);
	}

	println!("--- Recursion Diagnostic ---");
	println!("DEPTH: {best}");
	println!("LENGTH: {count}");

	item_
}
