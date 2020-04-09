use librsync_ffi::file::{Delta, Signature};
use reqwest::StatusCode;
use std::fs::{canonicalize, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::task::Context;
use std::thread;
use tokio::macros::support::{Future, Pin, Poll};
use tokio::task;
enum Message {
    FileChanged(PathBuf),
}

fn each_file<P: AsRef<Path>>(start_path: P, func: SyncSender<Message>) {
    let walker = walkdir::WalkDir::new(&start_path);

    for path in walker.into_iter() {
        let loc = path.unwrap().path().to_path_buf();
        if loc.is_file() {
            func.send(Message::FileChanged(loc.to_path_buf()));
        }
    }
}

#[tokio::main(max_threads = 32, core_threads = 8)]
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

    let client: Arc<reqwest::Client> = Arc::new(reqwest::Client::new());
    let (tx, rx) = sync_channel::<Message>(50);

    let thread_tx = tx.clone();
    let thread_target = target_path.clone();
    thread::spawn(move || {
        each_file(thread_target, thread_tx);
    });

    // ghetto ass concurrency limiter
    // The problem is that tokio will consume futures and start them as quickly as possible
    // without respecting concurrency limits. This suddenly opens thousands of tcp connections
    // either being killed by the OS or DOS-ing the server.
    // I need a better way to do this. Some kind of async worker pool queue

    let concurrency_count = Arc::new(Mutex::new(0 as usize));
    loop {
        let mesg = rx.recv().expect("Unable to get next");
        let client_ptr = client.clone();
        if let Message::FileChanged(path) = mesg {
            // open up the concurrency count, check if we have space otherwise wait for the current task
            {
                let mut count = concurrency_count
                    .lock()
                    .expect("Unable to get lock");
                *count += 1;
            }

            let concurrency_pointer = concurrency_count.clone();
            let handle = task::spawn(async move {
                sync_file(path, client_ptr).await;
                let mut count = concurrency_pointer
                    .lock()
                    .expect("unable to get inner lock");
                *count -= 1;
            });
            {
                let mut copy = 0;

                {
                    let mut count = concurrency_count
                        .lock()
                        .expect("Unable to get lock");
                    copy = *count;
                }

                if copy > 128 {
                    handle.await;
                }
            }
        }
    }
}

async fn sync_file<T: AsRef<Path>>(path: T, client: Arc<reqwest::Client>) {
    let signature = async {
        let req = client
            .get(&format!(
                "http://localhost:8000/request-signature?file_path={}",
                &path.as_ref().to_str().expect("Cannont create potato")
            ))
            .send()
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
            // println!(
            //     "Patching file: {:?}",
            //     &path.as_ref().to_str().expect("Unable to generate file")
            // );
            let payload = {
                let mut signature = Signature::from_bytes(signature);
                let path = std::env::current_dir().unwrap().join(&path);
                let (delta, _) = Delta::new(signature, &path);
                delta.to_bytes()
            };

            // if payload.len() <= 32 {
            //     println!("Files are identical, skipping");
            //     return;
            // };

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
            // println!(
            //     "Sending new file: {}",
            //     &path.as_ref().to_str().expect("Unable to generate file")
            // );
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
