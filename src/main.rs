use std::fs::File;
use std::path::PathBuf;
use std::process::exit;

use bytesize::ByteSize;
use colored::*;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use structopt::StructOpt;
use walkdir::WalkDir;

#[derive(StructOpt, Debug)]
#[structopt(name = "mx")]
struct Opt {
    #[structopt(short, long, default_value = "https://spin-archive.org")]
    host: String,

    #[structopt(long)]
    api_key: String,

    #[structopt(short, long)]
    tags: String,

    /// Files or directories to upload recursively.
    #[structopt(name = "FILE", parse(from_os_str))]
    paths: Vec<PathBuf>,
}

fn is_video(path: &PathBuf) -> bool {
    let guess = mime_guess::from_path(path);

    match guess.first() {
        Some(guess) => guess.to_string().starts_with("video/"),
        None => false,
    }
}

#[derive(Serialize, Debug)]
pub struct NewUploadRequest<'a> {
    file_name: &'a str,
    content_length: i64,
}

#[derive(Deserialize, Debug)]
pub struct NewUploadResponse {
    id: String,
    url: String,
}

#[derive(Deserialize, Debug)]
pub struct NewUploadError {
    status: String,
    reason: String,
}

fn begin_upload(
    host: &str,
    api_token: &str,
    path: &PathBuf,
) -> Result<NewUploadResponse, Box<dyn std::error::Error>> {
    let metadata = std::fs::metadata(&path).unwrap();
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let file_size = metadata.len() as i64;

    println!(
        "Uploading \"{}\" ({})",
        file_name,
        ByteSize(file_size as u64)
    );

    let new_upload_request = NewUploadRequest {
        file_name,
        content_length: file_size,
    };

    let endpoint = format!("{}/api/v1/uploads", host);

    let response = reqwest::blocking::Client::new()
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", api_token))
        .json(&new_upload_request)
        .send()?;

    if response.status() == 403 {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Invalid API key",
        )));
    }

    if response.status() == 400 {
        let json: NewUploadError = response.json()?;

        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            json.reason,
        )));
    }

    let json = response.json()?;

    Ok(json)
}

fn upload_file(path: &PathBuf, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path).unwrap();

    reqwest::blocking::Client::new()
        .put(url)
        .body(file)
        .send()?;

    Ok(())
}

#[derive(Serialize, Debug)]
pub struct FinalizeUploadRequest {
    id: String,
    tags: String,
    source: String,
    description: String,
    original_upload_date: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct FinalizeUploadResponse {
    id: String,
    url: String,
}

fn finalize_file(
    finalize_request: &FinalizeUploadRequest,
    host: &str,
    api_token: &str,
) -> Result<NewUploadResponse, Box<dyn std::error::Error>> {
    let endpoint = format!("{}/api/v1/uploads/finalize", host);

    let response = reqwest::blocking::Client::new()
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", api_token))
        .json(&finalize_request)
        .send()?;

    if response.status() == 403 {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Invalid API key",
        )));
    }

    let json = response.json()?;

    Ok(json)
}

fn main() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build_global()
        .unwrap();

    let opt = Opt::from_args();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut directories: Vec<PathBuf> = Vec::new();

    let host = opt.host.clone();
    let api_key = opt.api_key.clone();
    let tags = opt.tags.clone();

    for path in opt.paths.into_iter() {
        if path.is_dir() {
            directories.push(path);
        } else if path.is_file() {
            files.push(path);
        } else {
            eprintln!("Invalid file or directory: {}", path.to_str().unwrap());
            exit(1);
        }
    }

    for dir in directories.iter() {
        for entry in WalkDir::new(dir) {
            files.push(entry.unwrap().path().to_owned());
        }
    }

    files = files.into_iter().filter(|file| is_video(file)).collect();

    if files.len() == 0 {
        eprintln!("No video files found.");
        exit(1);
    }

    println!(
        "Uploading {} files with tags `{}`",
        files.len(),
        &opt.tags.bold()
    );

    files.into_par_iter().for_each(|file_path| {
        match begin_upload(&host, &api_key, &file_path)
            .and_then(|response| match upload_file(&file_path, &response.url) {
                Ok(_) => Ok(response),
                Err(err) => Err(err),
            })
            .and_then(|response| {
                let request = FinalizeUploadRequest {
                    id: response.id,
                    tags: tags.clone(),
                    source: "".to_owned(),
                    description: "".to_owned(),
                    original_upload_date: None,
                };

                finalize_file(&request, &host, &api_key)
            }) {
            Ok(response) => {
                println!(
                    "[{}] Uploaded: {}",
                    &file_path.to_str().unwrap(),
                    response.url
                );
            }
            Err(err) => {
                eprintln!("[{}] Error: {}", &file_path.to_str().unwrap(), err);
            }
        }
    })
}
