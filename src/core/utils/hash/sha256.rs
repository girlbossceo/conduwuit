use ring::{digest, digest::SHA256};

#[tracing::instrument(skip_all)]
pub(super) fn hash(keys: &[&[u8]]) -> Vec<u8> {
	// We only hash the pdu's event ids, not the whole pdu
	let bytes = keys.join(&0xFF);
	let hash = digest::digest(&SHA256, &bytes);
	hash.as_ref().to_owned()
}
