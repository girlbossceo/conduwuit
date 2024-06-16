#![cfg(feature = "console")]
use std::sync::Arc;

use conduit::{error, log, trace};
use ruma::events::room::message::RoomMessageEventContent;
use rustyline::{error::ReadlineError, history, Editor};
use termimad::MadSkin;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::services;

pub struct Console {
	join: Mutex<Option<JoinHandle<()>>>,
	input: Mutex<Editor<(), history::MemHistory>>,
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

	pub fn interrupt(self: &Arc<Self>) { Self::handle_interrupt(); }

	#[allow(clippy::let_underscore_must_use)]
	pub async fn close(self: &Arc<Self>) {
		if let Some(join) = self.join.lock().await.take() {
			_ = join.await;
		}
	}

	#[tracing::instrument(skip_all, name = "console")]
	async fn worker(self: Arc<Self>) {
		while services().server.running() {
			let mut input = self.input.lock().await;

			let suppression = log::Suppress::new(&services().server);
			let line = tokio::task::block_in_place(|| input.readline("uwu> "));
			drop(suppression);

			match line {
				Ok(string) => self.handle(string).await,
				Err(e) => match e {
					ReadlineError::Eof => break,
					ReadlineError::Interrupted => Self::handle_interrupt(),
					ReadlineError::WindowResized => Self::handle_winch(),
					_ => error!("console: {e:?}"),
				},
			}
		}

		self.join.lock().await.take();
	}

	async fn handle(&self, line: String) {
		if line.is_empty() {
			return;
		}

		match services().admin.command_in_place(line, None).await {
			Ok(Some(content)) => self.output(content).await,
			Err(e) => error!("processing command: {e}"),
			_ => (),
		}
	}

	async fn output(&self, output_content: RoomMessageEventContent) {
		let output = self.output.term_text(output_content.body());
		println!("{output}");
	}

	fn handle_interrupt() {
		trace!("interrupted");
	}

	fn handle_winch() {
		trace!("winch");
	}
}
