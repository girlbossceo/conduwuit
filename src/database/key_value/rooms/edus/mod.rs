mod presence;

use crate::{database::KeyValueDatabase, service};

impl service::rooms::edus::Data for KeyValueDatabase {}
