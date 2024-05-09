use infer::MatcherType;

/// Returns a Content-Disposition of `attachment` or `inline`, depending on the
/// *parsed* contents of the file uploaded via format magic keys using `infer`
/// crate (basically libmagic without needing libmagic).
///
/// This forbids trusting what the client or remote server says the file is from
/// their `Content-Type` and we try to detect it ourselves. Also returns
/// `attachment` if the Content-Type does not match what we detected.
///
/// TODO: add a "strict" function for comparing the Content-Type with what we
/// detected: `file_type.mime_type() != content_type`
pub(crate) fn content_disposition_type(buf: &[u8], _content_type: &Option<String>) -> &'static str {
	let Some(file_type) = infer::get(buf) else {
		return "attachment";
	};

	match file_type.matcher_type() {
		MatcherType::Image | MatcherType::Audio | MatcherType::Text | MatcherType::Video => "inline",
		_ => "attachment",
	}
}

/// sanitises the file name for the Content-Disposition using
/// `sanitize_filename` crate
#[tracing::instrument]
pub(crate) fn sanitise_filename(filename: String) -> String {
	let options = sanitize_filename::Options {
		truncate: false,
		..Default::default()
	};

	sanitize_filename::sanitize_with_options(filename, options)
}

/// creates the final Content-Disposition based on whether the filename exists
/// or not.
///
/// if filename exists: `Content-Disposition: attachment/inline;
/// filename=filename.ext` else: `Content-Disposition: attachment/inline`
#[tracing::instrument(skip(file))]
pub(crate) fn make_content_disposition(
	file: &[u8], content_type: &Option<String>, content_disposition: Option<String>,
) -> String {
	let filename = content_disposition.map_or_else(String::new, |content_disposition| {
		let (_, filename) = content_disposition
			.split_once("filename=")
			.unwrap_or(("", ""));

		if filename.is_empty() {
			String::new()
		} else {
			sanitise_filename(filename.to_owned())
		}
	});

	if !filename.is_empty() {
		// Content-Disposition: attachment/inline; filename=filename.ext
		format!("{}; filename={}", content_disposition_type(file, content_type), filename)
	} else {
		// Content-Disposition: attachment/inline
		String::from(content_disposition_type(file, content_type))
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn string_sanitisation() {
		const SAMPLE: &str =
			"🏳️‍⚧️this\\r\\n įs \r\\n ä \\r\nstrïng 🥴that\n\r ../../../../../../../may be\r\n malicious🏳️‍⚧️";
		const SANITISED: &str = "🏳️‍⚧️thisrn įs n ä rstrïng 🥴that ..............may be malicious🏳️‍⚧️";

		let options = sanitize_filename::Options {
			windows: true,
			truncate: true,
			replacement: "",
		};

		// cargo test -- --nocapture
		println!("{}", SAMPLE);
		println!("{}", sanitize_filename::sanitize_with_options(SAMPLE, options.clone()));
		println!("{:?}", SAMPLE);
		println!("{:?}", sanitize_filename::sanitize_with_options(SAMPLE, options.clone()));

		assert_eq!(SANITISED, sanitize_filename::sanitize_with_options(SAMPLE, options.clone()));
	}
}
