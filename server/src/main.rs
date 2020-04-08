#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
use rocket::http::RawStr;
mod pathtools;

use crate::pathtools::get_temporary_directory;
use librsync_ffi::file::{patch_file, Delta, Signature};
use rocket::Data;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

#[get("/request-signature?<file_path>")]
fn request_signature(file_path: &RawStr) -> Option<Vec<u8>> {
    let path = get_temporary_directory().join(&file_path.to_string());
    if path.exists() {
        let (signature, stats) = Signature::new(&path).expect("Unable to get signature for file");
        Some(signature.into_bytes())
    } else {
        None
    }
}

#[post("/patch-file?<file_path>", data = "<data>")]
fn apply_patch(file_path: &RawStr, data: Data) -> Option<&'static str> {
    let path = get_temporary_directory().join(&file_path.to_string());
    let mut file = match &path.exists() {
        true => OpenOptions::new().read(true).write(true).open(&path),
        false => File::create(&path),
    }
    .expect("Unable to open file");

    let mut buffer = Vec::new();
    data.stream_to(&mut buffer);

    let delta = Delta::from_bytes(buffer);
    let new_file_contents =
        patch_file(&path.to_str().expect("Unable to create path string"), delta);

    file.set_len(0);
    file.write_all(new_file_contents.as_slice());

    Some("Ok")
}

#[post("/new-file?<file_path>", data = "<data>")]
fn new_file(file_path: &RawStr, data: Data) -> Option<&'static str> {
    let path = get_temporary_directory().join(&file_path.to_string());
    create_dir_all(&path.parent().expect("Creating directories failed"));

    File::create(&path).expect("Unable to create file");
    data.stream_to_file(&path);
    Some("Ok")
}

fn main() {
    rocket::ignite()
        .mount("/", routes![request_signature, apply_patch, new_file])
        .launch();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket::local::Client;

    #[test]
    fn it_serializes_a_signature_correctly() {
        let rocket = rocket::ignite().mount("/", routes![request_signature, apply_patch, new_file]);
        let client = Client::new(rocket).unwrap();

        // create a sample file in our test directory
        let file = get_temporary_directory().join("some-file");
        {
            let mut file_handle = File::create(&file).unwrap();
            file_handle.write_all(&"some data to write".as_bytes());
            file_handle.sync_all();
        }

        let request = client.get("/request-signature?file_path=some-file");

        let mut response = request.dispatch();
        let bytes = response.body_bytes().unwrap();
        dbg!(String::from_utf8_lossy(&bytes.as_slice()));
        let signature = Signature::from_bytes(bytes);
    }
}
