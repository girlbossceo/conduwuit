use std::cmp;

use proc_macro::TokenStream;
use quote::ToTokens;
use syn::{Item, Meta};

use crate::Result;

pub(super) fn recursion_depth(item: Item, _args: &[Meta]) -> Result<TokenStream> {
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

	Ok(item.into_token_stream().into())
}
