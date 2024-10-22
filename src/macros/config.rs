use std::{
	collections::{HashMap, HashSet},
	fmt::Write as _,
	fs::OpenOptions,
	io::Write as _,
};

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::ToTokens;
use syn::{
	parse::Parser, punctuated::Punctuated, spanned::Spanned, Error, Expr, ExprLit, Field, Fields, FieldsNamed,
	ItemStruct, Lit, Meta, MetaList, MetaNameValue, Type, TypePath,
};

use crate::{utils::is_cargo_build, Result};

const UNDOCUMENTED: &str = "# This item is undocumented. Please contribute documentation for it.";

#[allow(clippy::needless_pass_by_value)]
pub(super) fn example_generator(input: ItemStruct, args: &[Meta]) -> Result<TokenStream> {
	if is_cargo_build() {
		generate_example(&input, args)?;
	}

	Ok(input.to_token_stream().into())
}

#[allow(clippy::needless_pass_by_value)]
#[allow(unused_variables)]
fn generate_example(input: &ItemStruct, args: &[Meta]) -> Result<()> {
	let settings = get_settings(args);

	let filename = settings
		.get("filename")
		.ok_or_else(|| Error::new(args[0].span(), "missing required 'filename' attribute argument"))?;

	let undocumented = settings
		.get("undocumented")
		.map_or(UNDOCUMENTED, String::as_str);

	let ignore: HashSet<&str> = settings
		.get("ignore")
		.map_or("", String::as_str)
		.split(' ')
		.collect();

	let section = settings
		.get("section")
		.ok_or_else(|| Error::new(args[0].span(), "missing required 'section' attribute argument"))?;

	let mut file = OpenOptions::new()
		.write(true)
		.create(section == "global")
		.truncate(section == "global")
		.append(section != "global")
		.open(filename)
		.map_err(|e| Error::new(Span::call_site(), format!("Failed to open config file for generation: {e}")))?;

	if let Some(header) = settings.get("header") {
		file.write_all(header.as_bytes())
			.expect("written to config file");
	}

	file.write_fmt(format_args!("\n[{section}]\n"))
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

			if ignore.contains(ident.to_string().as_str()) {
				continue;
			}

			let Some(type_name) = get_type_name(field) else {
				continue;
			};

			let doc = get_doc_comment(field)
				.unwrap_or_else(|| undocumented.into())
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

	if let Some(footer) = settings.get("footer") {
		file.write_all(footer.as_bytes())
			.expect("written to config file");
	}

	Ok(())
}

fn get_settings(args: &[Meta]) -> HashMap<String, String> {
	let mut map = HashMap::new();
	for arg in args {
		let Meta::NameValue(MetaNameValue {
			path,
			value,
			..
		}) = arg
		else {
			continue;
		};

		let Expr::Lit(
			ExprLit {
				lit: Lit::Str(str),
				..
			},
			..,
		) = value
		else {
			continue;
		};

		let Some(key) = path.segments.iter().next().map(|s| s.ident.clone()) else {
			continue;
		};

		map.insert(key.to_string(), str.value());
	}

	map
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
