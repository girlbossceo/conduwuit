use std::{borrow::Cow, collections::BTreeMap, ops::Deref};

use clap::Subcommand;
use conduwuit::{
	apply, at, is_zero,
	utils::{
		stream::{ReadyExt, TryIgnore, TryParallelExt},
		string::EMPTY,
		IterStream,
	},
	Err, Result,
};
use futures::{FutureExt, StreamExt, TryStreamExt};
use ruma::events::room::message::RoomMessageEventContent;
use tokio::time::Instant;

use crate::{admin_command, admin_command_dispatch};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
#[allow(clippy::enum_variant_names)]
/// Query tables from database
pub(crate) enum RawCommand {
	/// - List database maps
	RawMaps,

	/// - Raw database query
	RawGet {
		/// Map name
		map: String,

		/// Key
		key: String,
	},

	/// - Raw database keys iteration
	RawKeys {
		/// Map name
		map: String,

		/// Key prefix
		prefix: Option<String>,
	},

	/// - Raw database key size breakdown
	RawKeysSizes {
		/// Map name
		map: Option<String>,

		/// Key prefix
		prefix: Option<String>,
	},

	/// - Raw database keys total bytes
	RawKeysTotal {
		/// Map name
		map: Option<String>,

		/// Key prefix
		prefix: Option<String>,
	},

	/// - Raw database values size breakdown
	RawValsSizes {
		/// Map name
		map: Option<String>,

		/// Key prefix
		prefix: Option<String>,
	},

	/// - Raw database values total bytes
	RawValsTotal {
		/// Map name
		map: Option<String>,

		/// Key prefix
		prefix: Option<String>,
	},

	/// - Raw database items iteration
	RawIter {
		/// Map name
		map: String,

		/// Key prefix
		prefix: Option<String>,
	},

	/// - Raw database keys iteration
	RawKeysFrom {
		/// Map name
		map: String,

		/// Lower-bound
		start: String,

		/// Limit
		#[arg(short, long)]
		limit: Option<usize>,
	},

	/// - Raw database items iteration
	RawIterFrom {
		/// Map name
		map: String,

		/// Lower-bound
		start: String,

		/// Limit
		#[arg(short, long)]
		limit: Option<usize>,
	},

	/// - Raw database record count
	RawCount {
		/// Map name
		map: Option<String>,

		/// Key prefix
		prefix: Option<String>,
	},

	/// - Compact database
	Compact {
		#[arg(short, long, alias("column"))]
		map: Option<Vec<String>>,

		#[arg(long)]
		start: Option<String>,

		#[arg(long)]
		stop: Option<String>,

		#[arg(long)]
		from: Option<usize>,

		#[arg(long)]
		into: Option<usize>,

		/// There is one compaction job per column; then this controls how many
		/// columns are compacted in parallel. If zero, one compaction job is
		/// still run at a time here, but in exclusive-mode blocking any other
		/// automatic compaction jobs until complete.
		#[arg(long)]
		parallelism: Option<usize>,

		#[arg(long, default_value("false"))]
		exhaustive: bool,
	},
}

#[admin_command]
pub(super) async fn compact(
	&self,
	map: Option<Vec<String>>,
	start: Option<String>,
	stop: Option<String>,
	from: Option<usize>,
	into: Option<usize>,
	parallelism: Option<usize>,
	exhaustive: bool,
) -> Result<RoomMessageEventContent> {
	use conduwuit_database::compact::Options;

	let default_all_maps = map
		.is_none()
		.then(|| {
			self.services
				.db
				.keys()
				.map(Deref::deref)
				.map(ToOwned::to_owned)
		})
		.into_iter()
		.flatten();

	let maps: Vec<_> = map
		.unwrap_or_default()
		.into_iter()
		.chain(default_all_maps)
		.map(|map| self.services.db.get(&map))
		.filter_map(Result::ok)
		.cloned()
		.collect();

	if maps.is_empty() {
		return Err!("--map argument invalid. not found in database");
	}

	let range = (
		start.as_ref().map(String::as_bytes).map(Into::into),
		stop.as_ref().map(String::as_bytes).map(Into::into),
	);

	let options = Options {
		range,
		level: (from, into),
		exclusive: parallelism.is_some_and(is_zero!()),
		exhaustive,
	};

	let runtime = self.services.server.runtime().clone();
	let parallelism = parallelism.unwrap_or(1);
	let results = maps
		.into_iter()
		.try_stream()
		.paralleln_and_then(runtime, parallelism, move |map| {
			map.compact_blocking(options.clone())?;
			Ok(map.name().to_owned())
		})
		.collect::<Vec<_>>();

	let timer = Instant::now();
	let results = results.await;
	let query_time = timer.elapsed();
	self.write_str(&format!("Jobs completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"))
		.await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_count(
	&self,
	map: Option<String>,
	prefix: Option<String>,
) -> Result<RoomMessageEventContent> {
	let prefix = prefix.as_deref().unwrap_or(EMPTY);

	let default_all_maps = map
		.is_none()
		.then(|| self.services.db.keys().map(Deref::deref))
		.into_iter()
		.flatten();

	let maps: Vec<_> = map
		.iter()
		.map(String::as_str)
		.chain(default_all_maps)
		.map(|map| self.services.db.get(map))
		.filter_map(Result::ok)
		.cloned()
		.collect();

	let timer = Instant::now();
	let count = maps
		.iter()
		.stream()
		.then(|map| map.raw_count_prefix(&prefix))
		.ready_fold(0_usize, usize::saturating_add)
		.await;

	let query_time = timer.elapsed();
	self.write_str(&format!("Query completed in {query_time:?}:\n\n```rs\n{count:#?}\n```"))
		.await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_keys(
	&self,
	map: String,
	prefix: Option<String>,
) -> Result<RoomMessageEventContent> {
	writeln!(self, "```").boxed().await?;

	let map = self.services.db.get(map.as_str())?;
	let timer = Instant::now();
	prefix
		.as_deref()
		.map_or_else(|| map.raw_keys().boxed(), |prefix| map.raw_keys_prefix(prefix).boxed())
		.map_ok(String::from_utf8_lossy)
		.try_for_each(|str| writeln!(self, "{str:?}"))
		.boxed()
		.await?;

	let query_time = timer.elapsed();
	let out = format!("\n```\n\nQuery completed in {query_time:?}");
	self.write_str(out.as_str()).await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_keys_sizes(
	&self,
	map: Option<String>,
	prefix: Option<String>,
) -> Result<RoomMessageEventContent> {
	let prefix = prefix.as_deref().unwrap_or(EMPTY);

	let default_all_maps = map
		.is_none()
		.then(|| self.services.db.keys().map(Deref::deref))
		.into_iter()
		.flatten();

	let maps: Vec<_> = map
		.iter()
		.map(String::as_str)
		.chain(default_all_maps)
		.map(|map| self.services.db.get(map))
		.filter_map(Result::ok)
		.cloned()
		.collect();

	let timer = Instant::now();
	let result = maps
		.iter()
		.stream()
		.map(|map| map.raw_keys_prefix(&prefix))
		.flatten()
		.ignore_err()
		.map(<[u8]>::len)
		.ready_fold_default(|mut map: BTreeMap<_, usize>, len| {
			let entry = map.entry(len).or_default();
			*entry = entry.saturating_add(1);
			map
		})
		.await;

	let query_time = timer.elapsed();
	let result = format!("```\n{result:#?}\n```\n\nQuery completed in {query_time:?}");
	self.write_str(result.as_str()).await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_keys_total(
	&self,
	map: Option<String>,
	prefix: Option<String>,
) -> Result<RoomMessageEventContent> {
	let prefix = prefix.as_deref().unwrap_or(EMPTY);

	let default_all_maps = map
		.is_none()
		.then(|| self.services.db.keys().map(Deref::deref))
		.into_iter()
		.flatten();

	let maps: Vec<_> = map
		.iter()
		.map(String::as_str)
		.chain(default_all_maps)
		.map(|map| self.services.db.get(map))
		.filter_map(Result::ok)
		.cloned()
		.collect();

	let timer = Instant::now();
	let result = maps
		.iter()
		.stream()
		.map(|map| map.raw_keys_prefix(&prefix))
		.flatten()
		.ignore_err()
		.map(<[u8]>::len)
		.ready_fold_default(|acc: usize, len| acc.saturating_add(len))
		.await;

	let query_time = timer.elapsed();

	self.write_str(&format!("```\n{result:#?}\n\n```\n\nQuery completed in {query_time:?}"))
		.await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_vals_sizes(
	&self,
	map: Option<String>,
	prefix: Option<String>,
) -> Result<RoomMessageEventContent> {
	let prefix = prefix.as_deref().unwrap_or(EMPTY);

	let default_all_maps = map
		.is_none()
		.then(|| self.services.db.keys().map(Deref::deref))
		.into_iter()
		.flatten();

	let maps: Vec<_> = map
		.iter()
		.map(String::as_str)
		.chain(default_all_maps)
		.map(|map| self.services.db.get(map))
		.filter_map(Result::ok)
		.cloned()
		.collect();

	let timer = Instant::now();
	let result = maps
		.iter()
		.stream()
		.map(|map| map.raw_stream_prefix(&prefix))
		.flatten()
		.ignore_err()
		.map(at!(1))
		.map(<[u8]>::len)
		.ready_fold_default(|mut map: BTreeMap<_, usize>, len| {
			let entry = map.entry(len).or_default();
			*entry = entry.saturating_add(1);
			map
		})
		.await;

	let query_time = timer.elapsed();
	let result = format!("```\n{result:#?}\n```\n\nQuery completed in {query_time:?}");
	self.write_str(result.as_str()).await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_vals_total(
	&self,
	map: Option<String>,
	prefix: Option<String>,
) -> Result<RoomMessageEventContent> {
	let prefix = prefix.as_deref().unwrap_or(EMPTY);

	let default_all_maps = map
		.is_none()
		.then(|| self.services.db.keys().map(Deref::deref))
		.into_iter()
		.flatten();

	let maps: Vec<_> = map
		.iter()
		.map(String::as_str)
		.chain(default_all_maps)
		.map(|map| self.services.db.get(map))
		.filter_map(Result::ok)
		.cloned()
		.collect();

	let timer = Instant::now();
	let result = maps
		.iter()
		.stream()
		.map(|map| map.raw_stream_prefix(&prefix))
		.flatten()
		.ignore_err()
		.map(at!(1))
		.map(<[u8]>::len)
		.ready_fold_default(|acc: usize, len| acc.saturating_add(len))
		.await;

	let query_time = timer.elapsed();

	self.write_str(&format!("```\n{result:#?}\n\n```\n\nQuery completed in {query_time:?}"))
		.await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_iter(
	&self,
	map: String,
	prefix: Option<String>,
) -> Result<RoomMessageEventContent> {
	writeln!(self, "```").await?;

	let map = self.services.db.get(&map)?;
	let timer = Instant::now();
	prefix
		.as_deref()
		.map_or_else(|| map.raw_stream().boxed(), |prefix| map.raw_stream_prefix(prefix).boxed())
		.map_ok(apply!(2, String::from_utf8_lossy))
		.map_ok(apply!(2, Cow::into_owned))
		.try_for_each(|keyval| writeln!(self, "{keyval:?}"))
		.boxed()
		.await?;

	let query_time = timer.elapsed();
	self.write_str(&format!("\n```\n\nQuery completed in {query_time:?}"))
		.await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_keys_from(
	&self,
	map: String,
	start: String,
	limit: Option<usize>,
) -> Result<RoomMessageEventContent> {
	writeln!(self, "```").await?;

	let map = self.services.db.get(&map)?;
	let timer = Instant::now();
	map.raw_keys_from(&start)
		.map_ok(String::from_utf8_lossy)
		.take(limit.unwrap_or(usize::MAX))
		.try_for_each(|str| writeln!(self, "{str:?}"))
		.boxed()
		.await?;

	let query_time = timer.elapsed();
	self.write_str(&format!("\n```\n\nQuery completed in {query_time:?}"))
		.await?;

	Ok(RoomMessageEventContent::text_plain(""))
}

#[admin_command]
pub(super) async fn raw_iter_from(
	&self,
	map: String,
	start: String,
	limit: Option<usize>,
) -> Result<RoomMessageEventContent> {
	let map = self.services.db.get(&map)?;
	let timer = Instant::now();
	let result = map
		.raw_stream_from(&start)
		.map_ok(apply!(2, String::from_utf8_lossy))
		.map_ok(apply!(2, Cow::into_owned))
		.take(limit.unwrap_or(usize::MAX))
		.try_collect::<Vec<(String, String)>>()
		.await?;

	let query_time = timer.elapsed();
	Ok(RoomMessageEventContent::notice_markdown(format!(
		"Query completed in {query_time:?}:\n\n```rs\n{result:#?}\n```"
	)))
}

#[admin_command]
pub(super) async fn raw_get(&self, map: String, key: String) -> Result<RoomMessageEventContent> {
	let map = self.services.db.get(&map)?;
	let timer = Instant::now();
	let handle = map.get(&key).await?;
	let query_time = timer.elapsed();
	let result = String::from_utf8_lossy(&handle);

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"Query completed in {query_time:?}:\n\n```rs\n{result:?}\n```"
	)))
}

#[admin_command]
pub(super) async fn raw_maps(&self) -> Result<RoomMessageEventContent> {
	let list: Vec<_> = self.services.db.iter().map(at!(0)).copied().collect();

	Ok(RoomMessageEventContent::notice_markdown(format!("{list:#?}")))
}
