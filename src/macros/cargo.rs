use std::{fs::read_to_string, path::PathBuf};

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, AttributeArgs, ItemConst, Lit, NestedMeta};

pub(super) fn manifest(args: TokenStream, item: TokenStream) -> TokenStream {
	let item = parse_macro_input!(item as ItemConst);
	let args = parse_macro_input!(args as AttributeArgs);
	let member = args.into_iter().find_map(|arg| {
		let NestedMeta::Lit(arg) = arg else {
			return None;
		};
		let Lit::Str(arg) = arg else {
			return None;
		};
		Some(arg.value())
	});

	let path = manifest_path(member.as_deref());
	let manifest = read_to_string(&path).unwrap_or_default();

	let name = item.ident;
	let val = manifest.as_str();
	let ret = quote! {
		const #name: &'static str = #val;
	};

	ret.into()
}

#[allow(clippy::option_env_unwrap)]
fn manifest_path(member: Option<&str>) -> PathBuf {
	let mut path: PathBuf = option_env!("CARGO_MANIFEST_DIR")
		.expect("missing CARGO_MANIFEST_DIR in environment")
		.into();

	// conduwuit/src/macros/ -> conduwuit/src/
	path.pop();

	if let Some(member) = member {
		// conduwuit/$member/Cargo.toml
		path.push(member);
	} else {
		// conduwuit/src/ -> conduwuit/
		path.pop();
	}

	path.push("Cargo.toml");
	path
}
