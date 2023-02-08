use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, Write},
    path::Path,
    str::FromStr,
    time::Instant,
};

#[cfg(windows)]
use std::os::windows::prelude::OpenOptionsExt;

use anyhow::{Context, Result};
use clap::{arg, value_parser, Command};
use curl::easy::Easy;

#[derive(Clone)]
enum Source {
    List(HashMap<String, String>),
    Template(String),
}

impl FromStr for Source {
    type Err = io::Error;

    fn from_str(s: &str) -> io::Result<Self> {
        let path = Path::new(s);
        if path.exists() && path.is_file() {
            let br = BufReader::new(File::open(path)?);
            let mut map = HashMap::new();
            for line in br.lines() {
                let url = line?;
                let mut name = &url[..];
                if let Some((_, tail)) = name.rsplit_once('/') {
                    name = tail;
                }
                if let Some((head, _)) = name.split_once('?') {
                    name = head;
                }
                map.insert(name.into(), url);
            }
            Ok(Self::List(map))
        } else {
            Ok(Self::Template(s.into()))
        }
    }
}

impl Source {
    fn provide(&mut self, name: &str) -> Option<String> {
        match self {
            Self::List(map) => map.remove(name),
            Self::Template(template) => Some(template.replace("{}", name)),
        }
    }

    fn remove(&mut self, name: &str) {
        if let Self::List(map) = self {
            map.remove(name);
        }
    }

    fn into_rest(self) -> impl Iterator<Item = (String, String)> {
        match self {
            Self::List(map) => Some(map.into_iter()),
            Self::Template(_) => None,
        }
        .into_iter()
        .flatten()
    }
}

#[derive(Default)]
struct Counter {
    good: u32,
    bad: u32,
    na: u32,
    error: u32,
}

fn load_rec(
    file: &mut File,
    src: &mut Source,
    counter: &mut Counter,
) -> io::Result<HashSet<String>> {
    let mut reader = BufReader::new(file);
    let mut res = HashSet::new();
    let mut buf = String::new();
    while reader.read_line(&mut buf)? != 0 {
        if buf.ends_with('\n') {
            buf.pop();
            if buf.ends_with('\r') {
                buf.pop();
            }
        }
        if let Some((name, status)) = buf.split_once(": ") {
            res.insert(name.into());
            src.remove(name);
            match status {
                "good" => counter.good += 1,
                "bad" => counter.bad += 1,
                "n/a" => counter.na += 1,
                _ if status.starts_with("error") => counter.error += 1,
                _ => (),
            }
        }
        buf.clear();
    }
    Ok(res)
}

fn main() -> Result<()> {
    let mut matches = Command::new("howis")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(arg!(<FILE> ... "Files to check integrity of"))
        .arg(
            arg!(-s --src <SRC> "Source URL list file or template string")
                .required(true)
                .value_parser(value_parser!(Source)),
        )
        .arg(arg!(-r --rec <FILE> "Record file to resume progress from").default_value("howis.txt"))
        .arg(arg!(-u --user <USER> "Server username"))
        .arg(arg!(-p --pass <PASS> "Server password"))
        .get_matches_from(wild::args_os());

    let mut src = matches.remove_one::<Source>("src").unwrap();
    let mut counter = Counter::default();

    let rec = matches.get_one::<String>("rec").unwrap();
    let mut options = OpenOptions::new();

    #[cfg(windows)]
    options.share_mode(1);

    let mut rec = options
        .create(true)
        .read(true)
        .write(true)
        .open(rec)
        .context("failed to open record file")?;
    let rec_set = load_rec(&mut rec, &mut src, &mut counter)?;

    println!(
        "loaded: {} good, {} bad, {} n/a, {} error",
        counter.good, counter.bad, counter.na, counter.error
    );

    let mut handle = Easy::new();
    handle.follow_location(true).unwrap();
    handle.unrestricted_auth(true).unwrap();
    handle.cookie_file("").unwrap();
    if let Some(user) = matches.get_one::<String>("user") {
        handle.username(user).unwrap();
    }
    if let Some(pass) = matches.get_one::<String>("pass") {
        handle.password(pass).unwrap();
    }

    let mut buf = Box::new([0; 16384]);

    for path_str in matches.get_many::<String>("FILE").unwrap() {
        let path = Path::new(path_str);
        if !path.is_file() {
            println!("{path_str}: error: not a file");
            continue;
        }

        let name = path.file_name().unwrap().to_str().unwrap();
        if rec_set.contains(name) {
            continue;
        }
        print!("{name}: ");
        io::stdout().flush()?;

        let url = match src.provide(name) {
            Some(url) => url,
            None => {
                println!("error: missing source");
                writeln!(rec, "{name}: error: missing source")?;
                counter.error += 1;
                continue;
            }
        };

        let mut file = File::open(path)?;
        let mut good = true;
        let start = Instant::now();

        handle.url(&url).unwrap();
        let mut transfer = handle.transfer();
        transfer
            .write_function(|data| {
                let buf = &mut buf[..data.len()];
                if file.read_exact(buf).is_err() || data != buf {
                    good = false;
                }
                Ok(data.len())
            })
            .unwrap();

        if let Err(e) = transfer.perform() {
            println!("error: {e}");
            writeln!(rec, "{name}: error: {e}")?;
            counter.error += 1;
        } else {
            drop(transfer);
            let pos = file.stream_position()?;
            let len = file.metadata()?.len();
            if pos != len {
                good = false;
            }

            let good = if good {
                counter.good += 1;
                "good"
            } else {
                counter.bad += 1;
                "bad"
            };
            let speed = len as f64 / start.elapsed().as_secs_f64() / 1024.0;
            if speed >= 1024.0 {
                println!("{good} ({:.1} MB/s)", speed / 1024.0);
            } else {
                println!("{good} ({speed:.1} KB/s)");
            }
            writeln!(rec, "{name}: {good}")?;
        }
    }

    handle.nobody(true).unwrap();

    for (name, url) in src.into_rest() {
        print!("{name}: ");
        io::stdout().flush()?;

        handle.url(&url).unwrap();
        if let Err(e) = handle.perform() {
            println!("error: {e}");
            writeln!(rec, "{name}: error: {e}")?;
            counter.error += 1;
            continue;
        }

        let code = handle.response_code().unwrap();
        let eff_url = handle.effective_url().unwrap().unwrap();
        if code >= 200 && code < 300 && eff_url.contains(&name) {
            println!("error: available");
            writeln!(rec, "{name}: error: available")?;
            counter.error += 1;
        } else {
            println!("n/a");
            writeln!(rec, "{name}: n/a")?;
            counter.na += 1;
        }
    }

    println!(
        "finished: {} good, {} bad, {} n/a, {} error",
        counter.good, counter.bad, counter.na, counter.error
    );

    Ok(())
}
