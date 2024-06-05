use crate::debug_info;

const ATTACHMENT: &str = "attachment";
const INLINE: &str = "inline";

/// as defined by MSC2702
const ALLOWED_INLINE_CONTENT_TYPES: [&str; 26] = [
	// keep sorted
	"application/json",
	"application/ld+json",
	"audio/aac",
	"audio/flac",
	"audio/mp4",
	"audio/mpeg",
	"audio/ogg",
	"audio/wav",
	"audio/wave",
	"audio/webm",
	"audio/x-flac",
	"audio/x-pn-wav",
	"audio/x-wav",
	"image/apng",
	"image/avif",
	"image/gif",
	"image/jpeg",
	"image/png",
	"image/webp",
	"text/css",
	"text/csv",
	"text/plain",
	"video/mp4",
	"video/ogg",
	"video/quicktime",
	"video/webm",
];

/// Returns a Content-Disposition of `attachment` or `inline`, depending on the
/// Content-Type against MSC2702 list of safe inline Content-Types
/// (`ALLOWED_INLINE_CONTENT_TYPES`)
#[must_use]
pub fn content_disposition_type(content_type: &Option<String>) -> &'static str {
	let Some(content_type) = content_type else {
		debug_info!("No Content-Type was given, assuming attachment for Content-Disposition");
		return ATTACHMENT;
	};

	// is_sorted is unstable
	/* debug_assert!(ALLOWED_INLINE_CONTENT_TYPES.is_sorted(),
	 * "ALLOWED_INLINE_CONTENT_TYPES is not sorted"); */

	let content_type = content_type
		.split(';')
		.next()
		.unwrap_or(content_type)
		.to_ascii_lowercase();

	if ALLOWED_INLINE_CONTENT_TYPES
		.binary_search(&content_type.as_str())
		.is_ok()
	{
		INLINE
	} else {
		ATTACHMENT
	}
}

/// sanitises the file name for the Content-Disposition using
/// `sanitize_filename` crate
#[tracing::instrument]
pub fn sanitise_filename(filename: String) -> String {
	let options = sanitize_filename::Options {
		truncate: false,
		..Default::default()
	};

	sanitize_filename::sanitize_with_options(filename, options)
}

/// creates the final Content-Disposition based on whether the filename exists
/// or not, or if a requested filename was specified (media download with
/// filename)
///
/// if filename exists:
/// `Content-Disposition: attachment/inline; filename=filename.ext`
///
/// else: `Content-Disposition: attachment/inline`
pub fn make_content_disposition(
	content_type: &Option<String>, content_disposition: Option<String>, req_filename: Option<String>,
) -> String {
	let filename: String;

	if let Some(req_filename) = req_filename {
		filename = sanitise_filename(req_filename);
	} else {
		filename = content_disposition.map_or_else(String::new, |content_disposition| {
			let (_, filename) = content_disposition
				.split_once("filename=")
				.unwrap_or(("", ""));

			if filename.is_empty() {
				String::new()
			} else {
				sanitise_filename(filename.to_owned())
			}
		});
	};

	if !filename.is_empty() {
		// Content-Disposition: attachment/inline; filename=filename.ext
		format!("{}; filename={}", content_disposition_type(content_type), filename)
	} else {
		// Content-Disposition: attachment/inline
		String::from(content_disposition_type(content_type))
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
		println!("{SAMPLE}");
		println!("{}", sanitize_filename::sanitize_with_options(SAMPLE, options.clone()));
		println!("{SAMPLE:?}");
		println!("{:?}", sanitize_filename::sanitize_with_options(SAMPLE, options.clone()));

		assert_eq!(SANITISED, sanitize_filename::sanitize_with_options(SAMPLE, options.clone()));
	}
}
