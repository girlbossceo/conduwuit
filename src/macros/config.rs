use std::fmt::Write;

use proc_macro::TokenStream;
use quote::ToTokens;
use syn::{Expr, ExprLit, Field, Fields, FieldsNamed, ItemStruct, Lit, Meta, MetaNameValue, Type, TypePath};

use crate::{utils::is_cargo_build, Result};

#[allow(clippy::needless_pass_by_value)]
pub(super) fn example_generator(input: ItemStruct, args: &[Meta]) -> Result<TokenStream> {
	if is_cargo_build() {
		generate_example(&input, args)?;
	}

	Ok(input.to_token_stream().into())
}

#[allow(clippy::needless_pass_by_value)]
#[allow(unused_variables)]
fn generate_example(input: &ItemStruct, _args: &[Meta]) -> Result<()> {
	if let Fields::Named(FieldsNamed {
		named,
		..
	}) = &input.fields
	{
		for field in named {
			let Some(ident) = &field.ident else {
				continue;
			};

			let Some(doc) = get_doc_comment(field) else {
				continue;
			};

			let Some(type_name) = get_type_name(field) else {
				continue;
			};

			//println!("{:?} {type_name:?}\n{doc}", ident.to_string());
		}
	}

	Ok(())
}

fn get_doc_comment(field: &Field) -> Option<String> {
	let mut out = String::new();
	for attr in &field.attrs {
		let Meta::NameValue(MetaNameValue {
			path,
			value,
			..
		}) = &attr.meta
		else {
			continue;
		};

		if !path
			.segments
			.iter()
			.next()
			.is_some_and(|s| s.ident == "doc")
		{
			continue;
		}

		let Expr::Lit(ExprLit {
			lit,
			..
		}) = &value
		else {
			continue;
		};

		let Lit::Str(token) = &lit else {
			continue;
		};

		writeln!(&mut out, "# {}", token.value()).expect("wrote to output string buffer");
	}

	(!out.is_empty()).then_some(out)
}

fn get_type_name(field: &Field) -> Option<String> {
	let Type::Path(TypePath {
		path,
		..
	}) = &field.ty
	else {
		return None;
	};

	path.segments
		.iter()
		.next()
		.map(|segment| segment.ident.to_string())
}
