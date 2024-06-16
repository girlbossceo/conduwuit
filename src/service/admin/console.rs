#![cfg(feature = "console")]
use std::sync::Arc;

use conduit::{debug, defer, error, log, trace};
use futures_util::future::{AbortHandle, Abortable};
use ruma::events::room::message::RoomMessageEventContent;
use rustyline::{error::ReadlineError, history, Editor};
use termimad::MadSkin;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::services;

pub struct Console {
	join: Mutex<Option<JoinHandle<()>>>,
	input: Mutex<Editor<(), history::MemHistory>>,
	abort: std::sync::Mutex<Option<AbortHandle>>,
	output: MadSkin,
}

impl Console {
	#[must_use]
	pub fn new() -> Arc<Self> {
		use rustyline::config::{Behavior, BellStyle};
		use termimad::{crossterm::style::Color, Alignment, CompoundStyle, LineStyle};

		let config = rustyline::Config::builder()
			.enable_signals(false)
			.behavior(Behavior::PreferTerm)
			.bell_style(BellStyle::Visible)
			.auto_add_history(true)
			.max_history_size(100)
			.expect("valid history size")
			.indent_size(4)
			.tab_stop(4)
			.build();

		let history = history::MemHistory::with_config(config);
		let input = Editor::with_history(config, history).expect("builder configuration succeeded");

		let mut output = MadSkin::default_dark();

		let code_style = CompoundStyle::with_fgbg(Color::AnsiValue(40), Color::AnsiValue(234));
		output.inline_code = code_style.clone();
		output.code_block = LineStyle {
			left_margin: 0,
			right_margin: 0,
			align: Alignment::Left,
			compound_style: code_style,
		};

		Arc::new(Self {
			join: None.into(),
			input: Mutex::new(input),
			abort: None.into(),
			output,
		})
	}

	#[allow(clippy::let_underscore_must_use)]
	pub async fn start(self: &Arc<Self>) {
		let mut join = self.join.lock().await;
		if join.is_none() {
			let self_ = Arc::clone(self);
			_ = join.insert(services().server.runtime().spawn(self_.worker()));
		}
	}

	#[allow(clippy::let_underscore_must_use)]
	pub async fn close(self: &Arc<Self>) {
		if let Some(join) = self.join.lock().await.take() {
			_ = join.await;
		}
	}

	pub fn interrupt(self: &Arc<Self>) {
		if let Some(abort) = self.abort.lock().expect("locked").take() {
			debug!("Interrupting console command...");
			abort.abort();
		}
	}

	#[tracing::instrument(skip_all, name = "console")]
	async fn worker(self: Arc<Self>) {
		debug!("session starting");
		while services().server.running() {
			let mut input = self.input.lock().await;

			let suppression = log::Suppress::new(&services().server);
			let line = tokio::task::block_in_place(|| input.readline("uwu> "));
			drop(suppression);

			trace!(?line, "input");
			match line {
				Ok(string) => self.clone().handle(string).await,
				Err(e) => match e {
					ReadlineError::Interrupted | ReadlineError::Eof => break,
					ReadlineError::WindowResized => continue,
					_ => error!("console: {e:?}"),
				},
			}
		}

		debug!("session ending");
		self.join.lock().await.take();
	}

	#[allow(clippy::let_underscore_must_use)]
	async fn handle(self: Arc<Self>, line: String) {
		if line.is_empty() {
			return;
		}

		let future = self.clone().process(line);
		let (abort, abort_reg) = AbortHandle::new_pair();
		let future = Abortable::new(future, abort_reg);
		_ = self.abort.lock().expect("locked").insert(abort);
		defer! {{
			_ = self.abort.lock().expect("locked").take();
		}}

		_ = future.await;
	}

	async fn process(self: Arc<Self>, line: String) {
		match services().admin.command_in_place(line, None).await {
			Ok(Some(content)) => self.output(content).await,
			Err(e) => error!("processing command: {e}"),
			_ => (),
		}
	}

	async fn output(self: Arc<Self>, output_content: RoomMessageEventContent) {
		let output = self.output.term_text(output_content.body());
		println!("{output}");
	}
}
