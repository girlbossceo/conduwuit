use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, Meta, MetaList};

use crate::Result;

pub(super) fn implement(item: ItemFn, args: &[Meta]) -> Result<TokenStream> {
	let Meta::List(MetaList {
		path,
		..
	}) = &args
		.first()
		.expect("missing path to trait or item to implement")
	else {
		panic!("invalid path to item for implement");
	};

	let input = item;
	let out = quote! {
		impl #path {
			#input
		}
	};

	Ok(out.into())
}
