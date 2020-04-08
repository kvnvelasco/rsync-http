use notify::{poll, DebouncedEvent};
use notify::{RecommendedWatcher, RecursiveMode, Result, Watcher};
use std::fs::{canonicalize, File};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::thread::spawn;
use std::time::Duration;

use librsync_ffi::file::{Delta, Signature};
use reqwest::StatusCode;
use std::borrow::Borrow;
use std::io::{Bytes, Read};
use std::sync::Arc;
use std::thread;
use tokio;

fn watch() -> notify::Result<()> {
    let mut args = std::env::args();

    let mut target_path = PathBuf::from(&args.nth(1).unwrap_or(".".to_owned()));

    let current_dir = std::env::current_dir().expect("Unable to get current dir");

    if target_path.is_absolute() {
        panic!("Absolute paths cannot be monitored.")
    } else {
        target_path = current_dir.join(target_path);
    }

    {
        let absolute_target = canonicalize(&target_path).expect("Invalid path");
        if !absolute_target.starts_with(&current_dir) {
            panic!("You can only watch child directories");
        }
        target_path = absolute_target;
    }

    if !target_path.exists() {
        panic!("Target path does not exist");
    }
    // Create a channel to receive the events.
    let (tx, rx) = channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_millis(500))?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(
        target_path.to_str().expect("cannot make cwd"),
        RecursiveMode::Recursive,
    )?;

    // This is a simple loop, but you may want to use more complex logic here,
    // for example to handle I/O.
    loop {
        match rx.recv() {
            Ok(event) => {
                if let DebouncedEvent::Write(ev) = &event {
                    let path = ev
                        .strip_prefix(&current_dir)
                        .expect("Unable to strip prefix")
                        .to_owned();
                    tokio::spawn(async move { sync_file(path).await });
                }
            }
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

async fn sync_file<T: AsRef<Path>>(path: T) {
    let signature = async {
        let req = reqwest::get(&format!(
            "http://localhost:8000/request-signature?file_path={}",
            &path.as_ref().to_str().expect("Cannont create potato")
        ))
        .await
        .expect("Unable to get response");

        let status: StatusCode = req.status();

        if status.as_str() == "404" {
            None
        } else {
            let bytes = req.bytes().await.expect("Unable to get body");
            if bytes.len() > 0 {
                Some(Vec::<u8>::from(bytes.as_ref()))
            } else {
                None
            }
        }
    }
    .await;

    match signature {
        Some(signature) => {
            println!(
                "Patching file: {:?}",
                &path.as_ref().to_str().expect("Unable to generate file")
            );
            let payload = {
                let mut signature = Signature::from_bytes(signature);
                let path = std::env::current_dir().unwrap().join(&path);
                let (delta, _) = Delta::new(&mut signature, &path);
                delta.to_bytes()
            };

            if payload.len() <= 32 {
                println!("Files are identical, skipping");
                return;
            };

            let client = reqwest::Client::new();
            client
                .post(&format!(
                    "http://localhost:8000/patch-file?file_path={}",
                    &path.as_ref().to_str().expect("Unable to generate file")
                ))
                .body(payload)
                .send()
                .await;
        }
        None => {
            println!(
                "Sending new file: {}",
                &path.as_ref().to_str().expect("Unable to generate file")
            );
            let client = reqwest::Client::new();
            let mut file = File::open(std::env::current_dir().unwrap().join(&path))
                .expect("Unable to open file");
            let mut payload = Vec::new();
            file.read_to_end(&mut payload);
            client
                .post(&format!(
                    "http://localhost:8000/new-file?file_path={}",
                    &path.as_ref().to_str().expect("Unable to generate file")
                ))
                .body(payload)
                .send()
                .await;
        }
    };
}

fn each_file<P: AsRef<Path>, F: Fn(&Path)>(start_path: P, func: &F) {
    let reader = std::fs::read_dir(&start_path).expect("Unable to read directory");
    thread::sleep(Duration::from_millis(10));
    for entry in reader {
        let entry = entry.unwrap();
        if (entry.path().is_dir()) {
            each_file(&entry.path(), func.clone());
        } else {
            func(&entry.path())
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args();

    let mut target_path = PathBuf::from(&args.nth(1).unwrap_or(".".to_owned()));

    let current_dir = std::env::current_dir().expect("Unable to get current dir");

    if target_path.is_absolute() {
        panic!("Absolute paths cannot be monitored.")
    } else {
        target_path = current_dir.join(target_path);
    }

    {
        let absolute_target = canonicalize(&target_path).expect("Invalid path");
        if !absolute_target.starts_with(&current_dir) {
            panic!("You can only watch child directories");
        }
        target_path = absolute_target;
    }

    if !target_path.exists() {
        panic!("Target path does not exist");
    }

    each_file(&target_path, &|path| {
        let path = path
            .strip_prefix(&current_dir)
            .expect("Unable to strip prefix")
            .to_owned();

        tokio::spawn(async { sync_file(path).await });
    });
    // start watch
    watch().expect("Unable to watch directory");
}
