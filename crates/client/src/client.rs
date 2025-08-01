use async_recursion::async_recursion;
use charon_lib::event::{DomainEvent, Event};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, Stdout},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{
        UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
};
use tracing::{error, info, warn};

use crate::{
    domain::{AppMsg, Command},
    root::AppManager,
    tui::{resume_tui, suspend_tui},
};

pub struct CharonClient {
    app_mngr: AppManager,
    terminal: Terminal<CrosstermBackend<Stdout>>,
    reader: BufReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
    should_quit: bool,
}

impl CharonClient {
    pub fn new(app_mngr: AppManager, stream: UnixStream) -> Self {
        let terminal = ratatui::init();

        let (reader, writer) = stream.into_split();
        let writer = BufWriter::new(writer);
        let reader = BufReader::new(reader);

        Self {
            app_mngr,
            terminal,
            reader,
            writer,
            should_quit: false,
        }
    }

    pub async fn run(&mut self) -> io::Result<()> {
        info!("Client started");

        let mut line = String::new();
        let tick_duration = Duration::from_millis(500);
        let mut interval = tokio::time::interval(tick_duration);

        self.redraw()?;

        while !self.should_quit {
            tokio::select! {
                Ok(bytes) = self.reader.read_line(&mut line) => {
                    if bytes == 0 {
                        warn!("Connection closed by daemon");
                        self.should_quit = true;
                    } else {
                        match serde_json::from_str::<Event>(&line.trim()) {
                            Ok(event) => {
                                self.update_with_msg(&AppMsg::Backend(event.payload.clone())).await;
                            }
                            Err(e) => {
                                error!("Failed to parse event: {e}. Raw line: {:?}", line);
                            }
                        }
                    }
                    line.clear();
                }

                _ = interval.tick() => {
                    self.update_with_msg(&AppMsg::TimerTick(tick_duration)).await;
                }
            }
        }

        ratatui::restore();
        info!("Client quitting");
        Ok(())
    }

    fn redraw(&mut self) -> io::Result<()> {
        self.terminal.draw(|f| self.app_mngr.render(f))?;
        Ok(())
    }

    #[async_recursion]
    async fn update_with_msg(&mut self, msg: &AppMsg) {
        let cmd = self.app_mngr.update(msg).await;
        if let Some(cmd) = cmd {
            if let Err(err) = self.handle_command(&cmd).await {
                error!("Error while handling command: {err}");
            }
        }
    }

    async fn handle_command(&mut self, command: &Command) -> anyhow::Result<()> {
        match command {
            Command::Render => self.redraw()?,
            Command::SendEvent(event) => self.send(event).await?,
            Command::SuspendTUI => {
                suspend_tui(&mut self.terminal)?;
            }
            Command::ResumeTUI => {
                resume_tui(&mut self.terminal)?;
                self.terminal.clear()?;
                self.redraw()?;
            }
            Command::RunApp(app) => {
                if self.app_mngr.has_app(app) {
                    self.update_with_msg(&AppMsg::Deactivate).await;
                    self.app_mngr.set_active(app);
                    self.update_with_msg(&AppMsg::Activate).await;
                    self.redraw()?;
                } else {
                    error!("Couldn't find app: {app}");
                }
            }
            Command::Exit => self.should_quit = true,
            // c => {
            //     warn!("Unhandled command: {:?}", c)
            // }
        }
        Ok(())
    }

    async fn send(&mut self, payload: &DomainEvent) -> anyhow::Result<()> {
        let event = Event::new("client".into(), payload.clone());
        let json = serde_json::to_string(&event)?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}
