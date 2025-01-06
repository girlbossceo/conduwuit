use std::fmt::Write;

use conduwuit::{implement, Result};

use super::Engine;

#[implement(Engine)]
pub fn file_list(&self) -> Result<String> {
	match self.db.live_files() {
		| Err(e) => Ok(String::from(e)),
		| Ok(mut files) => {
			files.sort_by_key(|f| f.name.clone());
			let mut res = String::new();
			writeln!(res, "| lev  | sst  | keys | dels | size | column |")?;
			writeln!(res, "| ---: | :--- | ---: | ---: | ---: | :---   |")?;
			for file in files {
				writeln!(
					res,
					"| {} | {:<13} | {:7}+ | {:4}- | {:9} | {} |",
					file.level,
					file.name,
					file.num_entries,
					file.num_deletions,
					file.size,
					file.column_family_name,
				)?;
			}

			Ok(res)
		},
	}
}
