use std::{fmt::Write as _, fs::File, io::Write as _};

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::ToTokens;
use syn::{
	parse::Parser, punctuated::Punctuated, Error, Expr, ExprLit, Field, Fields, FieldsNamed, ItemStruct, Lit, Meta,
	MetaList, MetaNameValue, Type, TypePath,
};

use crate::{utils::is_cargo_build, Result};

const UNDOCUMENTED: &str = "# This item is undocumented. Please contribute documentation for it.";
const HEADER: &str = "## Conduwuit Configuration\n##\n## THIS FILE IS GENERATED. Changes to documentation and \
                      defaults must\n## be made within the code found at src/core/config/\n";

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
	let mut file = File::create("conduwuit-example.toml")
		.map_err(|e| Error::new(Span::call_site(), format!("Failed to open config file for generation: {e}")))?;

	file.write_all(HEADER.as_bytes())
		.expect("written to config file");

	if let Fields::Named(FieldsNamed {
		named,
		..
	}) = &input.fields
	{
		for field in named {
			let Some(ident) = &field.ident else {
				continue;
			};

			let Some(type_name) = get_type_name(field) else {
				continue;
			};

			let doc = get_doc_comment(field)
				.unwrap_or_else(|| UNDOCUMENTED.into())
				.trim_end()
				.to_owned();

			let doc = if doc.ends_with('#') {
				format!("{doc}\n")
			} else {
				format!("{doc}\n#\n")
			};

			let default = get_doc_default(field)
				.or_else(|| get_default(field))
				.unwrap_or_default();

			let default = if !default.is_empty() {
				format!(" {default}")
			} else {
				default
			};

			file.write_fmt(format_args!("\n{doc}"))
				.expect("written to config file");

			file.write_fmt(format_args!("#{ident} ={default}\n"))
				.expect("written to config file");
		}
	}

	Ok(())
}

fn get_default(field: &Field) -> Option<String> {
	for attr in &field.attrs {
		let Meta::List(MetaList {
			path,
			tokens,
			..
		}) = &attr.meta
		else {
			continue;
		};

		if !path
			.segments
			.iter()
			.next()
			.is_some_and(|s| s.ident == "serde")
		{
			continue;
		}

		let Some(arg) = Punctuated::<Meta, syn::Token![,]>::parse_terminated
			.parse(tokens.clone().into())
			.ok()?
			.iter()
			.next()
			.cloned()
		else {
			continue;
		};

		match arg {
			Meta::NameValue(MetaNameValue {
				value: Expr::Lit(ExprLit {
					lit: Lit::Str(str),
					..
				}),
				..
			}) => {
				match str.value().as_str() {
					"HashSet::new" | "Vec::new" | "RegexSet::empty" => Some("[]".to_owned()),
					"true_fn" => return Some("true".to_owned()),
					_ => return None,
				};
			},
			Meta::Path {
				..
			} => return Some("false".to_owned()),
			_ => return None,
		};
	}

	None
}

fn get_doc_default(field: &Field) -> Option<String> {
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

		let value = token.value();
		if !value.trim().starts_with("default:") {
			continue;
		}

		return value
			.split_once(':')
			.map(|(_, v)| v)
			.map(str::trim)
			.map(ToOwned::to_owned);
	}

	None
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

		let value = token.value();
		if value.trim().starts_with("default:") {
			continue;
		}

		writeln!(&mut out, "#{value}").expect("wrote to output string buffer");
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
