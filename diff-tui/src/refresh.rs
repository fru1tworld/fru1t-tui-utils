use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
};

use anyhow::{Result, bail};

use crate::git::{Change, DiffLine, Mode, Repository, Snapshot, Sources};

#[derive(Default, PartialEq, Eq)]
pub struct Preview {
    pub patch: Vec<DiffLine>,
    pub sources: Sources,
    pub warning: Option<String>,
}

impl Preview {
    pub fn read(
        repo: &Repository,
        snapshot: &Snapshot,
        change: &Change,
        ignore_whitespace: bool,
    ) -> Result<Self> {
        let patch = repo.patch(snapshot, change, ignore_whitespace)?;
        let mut warning = None;
        let sources = if patch
            .iter()
            .any(|line| line.old.is_some() || line.new.is_some())
        {
            match repo.sources(snapshot, change) {
                Ok(sources) => sources,
                Err(error) => {
                    warning = Some(format!("Could not read source for highlighting: {error:#}"));
                    Sources::default()
                }
            }
        } else {
            Sources::default()
        };
        Ok(Self {
            patch,
            sources,
            warning,
        })
    }
}

pub struct Request {
    pub generation: u64,
    pub mode: Mode,
    pub selected: Option<Change>,
    pub ignore_whitespace: bool,
}

pub struct Update {
    pub snapshot: Snapshot,
    pub preview: Option<(Change, Preview)>,
}

impl Request {
    pub fn read(&self, repo: &Repository) -> Result<Update> {
        let snapshot = repo.snapshot(&self.mode)?;
        let preview = self
            .selected
            .as_ref()
            .and_then(|selected| {
                snapshot
                    .changes
                    .iter()
                    .find(|change| change.path == selected.path)
            })
            .map(|change| {
                Preview::read(repo, &snapshot, change, self.ignore_whitespace)
                    .map(|preview| (change.clone(), preview))
            })
            .transpose()?;
        Ok(Update { snapshot, preview })
    }
}

type Reply = (Request, Result<Update>);

pub struct Poller {
    requests: Sender<Request>,
    replies: Receiver<Reply>,
    pub running: bool,
}

impl Poller {
    pub fn new(root: PathBuf) -> Result<Self> {
        let (requests, pending) = mpsc::channel::<Request>();
        let (completed, replies) = mpsc::channel();
        thread::Builder::new()
            .name("git-refresh".into())
            .spawn(move || {
                let repo = Repository { root };
                for request in pending {
                    let update = request.read(&repo);
                    if completed.send((request, update)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            replies,
            running: false,
        })
    }

    pub fn request(&mut self, request: Request) -> Result<()> {
        if !self.running {
            self.requests.send(request)?;
            self.running = true;
        }
        Ok(())
    }

    pub fn take_ready(&mut self) -> Result<Option<Reply>> {
        match self.replies.try_recv() {
            Ok(reply) => {
                self.running = false;
                Ok(Some(reply))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => bail!("Git refresh worker stopped"),
        }
    }
}
