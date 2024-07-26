use proc_macro::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{parse_macro_input, AttributeArgs, FnArg::Typed, Ident, ItemFn, Pat, PatIdent, PatType, Stmt};

pub(super) fn refutable(args: TokenStream, input: TokenStream) -> TokenStream {
	let _args = parse_macro_input!(args as AttributeArgs);
	let mut item = parse_macro_input!(input as ItemFn);

	let inputs = item.sig.inputs.clone();
	let stmt = &mut item.block.stmts;
	let sig = &mut item.sig;
	for (i, input) in inputs.iter().enumerate() {
		let Typed(PatType {
			pat,
			..
		}) = input
		else {
			continue;
		};

		let Pat::Struct(ref pat) = **pat else {
			continue;
		};

		let variant = &pat.path;
		let fields = &pat.fields;

		// new versions of syn can replace this kronecker kludge with get_mut()
		for (j, input) in sig.inputs.iter_mut().enumerate() {
			if i != j {
				continue;
			}

			let Typed(PatType {
				ref mut pat,
				..
			}) = input
			else {
				continue;
			};

			let name = format!("_args_{i}");
			*pat = Box::new(Pat::Ident(PatIdent {
				ident: Ident::new(&name, Span::call_site().into()),
				attrs: Vec::new(),
				by_ref: None,
				mutability: None,
				subpat: None,
			}));

			let field = fields.iter();
			let refute = quote! {
				let #variant { #( #field ),*, .. } = #name else { panic!("incorrect variant passed to function argument {i}"); };
			};

			stmt.insert(0, syn::parse2::<Stmt>(refute).expect("syntax error"));
		}
	}

	item.into_token_stream().into()
}
