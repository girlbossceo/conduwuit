use crate::Result;
use ruma::RoomId;

type SearchPdusResult<'a> = Result<Option<(Box<dyn Iterator<Item = Vec<u8>> + 'a>, Vec<String>)>>;

pub trait Data: Send + Sync {
    fn index_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) -> Result<()>;

    fn search_pdus<'a>(&'a self, room_id: &RoomId, search_string: &str) -> SearchPdusResult<'a>;

    fn delete_all_search_tokenids_for_room(&self, room_id: &RoomId) -> Result<()>;
}
