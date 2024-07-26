use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, AttributeArgs, ItemFn, Meta, NestedMeta};

pub(super) fn implement(args: TokenStream, input: TokenStream) -> TokenStream {
	let args = parse_macro_input!(args as AttributeArgs);
	let item = parse_macro_input!(input as ItemFn);

	let NestedMeta::Meta(Meta::Path(receiver)) = args
		.first()
		.expect("missing path to trait or item to implement")
	else {
		panic!("invalid path to item for implement");
	};

	let out = quote! {
		impl #receiver {
			#item
		}
	};

	out.into_token_stream().into()
}
