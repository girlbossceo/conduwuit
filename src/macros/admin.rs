use conduit_core::utils::string::camel_to_snake_string;
use proc_macro::{Span, TokenStream};
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, AttributeArgs, Fields, Ident, ItemEnum, Variant};

pub(super) fn command_dispatch(args: TokenStream, input_: TokenStream) -> TokenStream {
	let input = input_.clone();
	let item = parse_macro_input!(input as ItemEnum);
	let _args = parse_macro_input!(args as AttributeArgs);
	let arm = item.variants.iter().map(dispatch_arm);
	let name = item.ident;
	let q = quote! {
		pub(super) async fn process(command: #name, body: Vec<&str>) -> Result<RoomMessageEventContent> {
			use #name::*;
			#[allow(non_snake_case)]
			Ok(match command {
				#( #arm )*
			})
		}
	};

	[input_, q.into()].into_iter().collect::<TokenStream>()
}

fn dispatch_arm(v: &Variant) -> TokenStream2 {
	let name = &v.ident;
	let target = camel_to_snake_string(&format!("{name}"));
	let handler = Ident::new(&target, Span::call_site().into());
	match &v.fields {
		Fields::Named(fields) => {
			let field = fields.named.iter().filter_map(|f| f.ident.as_ref());
			let arg = field.clone();
			quote! {
				#name { #( #field ),* } => Box::pin(#handler(body, #( #arg ),*)).await?,
			}
		},
		Fields::Unnamed(fields) => {
			let field = &fields.unnamed.first().expect("one field");
			quote! {
				#name ( #field ) => Box::pin(#handler::process(#field, body)).await?,
			}
		},
		Fields::Unit => {
			quote! {
				#name => Box::pin(#handler(body)).await?,
			}
		},
	}
}
