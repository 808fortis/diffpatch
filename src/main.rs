use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

type Result<T> = io::Result<T>;

#[derive(Clone)]
struct Blob {
    hash: String,
    content: Vec<u8>,
}

struct Index {
    files: Vec<(PathBuf, Blob)>,
}

impl Index {
    fn new() -> Self {
        Self { files: Vec::new() }
    }

    fn add_path(&mut self, path: &Path) -> Result<()> {
        let metadata = fs::metadata(path)?;
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                self.add_path(&entry.path())?;
            }
        } else if metadata.is_file() {
            let content = fs::read(path)?;
            let hash = format!("{:08x}", blake3::hash(&content).as_bytes()[0]);
            self.files.push((path.to_path_buf(), Blob { hash, content }));
        }
        Ok(())
    }

    fn save(&self, name: &str) -> Result<String> {
        fs::create_dir_all(".diffpatch")?;
        let hash = self.compute_root_hash();
        let data = serde_json::to_vec(&self.serialize())?;
        fs::write(format!(".diffpatch/{}_{}.idx", name, hash), &data)?;
        
        let mut index = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(".diffpatch/master")?;
        writeln!(index, "{}:{}", name, hash)?;
        Ok(hash)
    }

    fn compute_root_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        let mut sorted = self.files.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, blob) in sorted {
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(&[0]);
            hasher.update(blob.hash.as_bytes());
        }
        format!("{:08x}", hasher.finalize().as_bytes()[0])
    }

    fn serialize(&self) -> Vec<(String, Vec<u8>)> {
        self.files.iter()
            .map(|(p, b)| (p.to_string_lossy().to_string(), b.content.clone()))
            .collect()
    }

    fn load(name: &str, hash: &str) -> Result<Self> {
        let data = fs::read(format!(".diffpatch/{}_{}.idx", name, hash))?;
        let vec: Vec<(String, Vec<u8>)> = serde_json::from_slice(&data)?;
        let mut files = Vec::new();
        for (path, content) in vec {
            let hash = format!("{:08x}", blake3::hash(&content).as_bytes()[0]);
            files.push((PathBuf::from(path), Blob { hash, content }));
        }
        Ok(Self { files })
    }

    fn diff(&self, other: &Self) -> String {
        let mut out = String::new();
        
        let map1: std::collections::HashMap<_, _> = self.files.iter().map(|(p,b)| (p, b)).collect();
        let map2: std::collections::HashMap<_, _> = other.files.iter().map(|(p,b)| (p, b)).collect();
        
        let all_paths: HashSet<_> = map1.keys().chain(map2.keys()).collect();
        let mut sorted_paths: Vec<_> = all_paths.into_iter().collect();
        sorted_paths.sort();
        
        for path in sorted_paths {
            let b1 = map1.get(path);
            let b2 = map2.get(path);
            
            match (b1, b2) {
                (Some(o), Some(n)) if o.hash != n.hash => {
                    out.push_str(&format!("--- a/{}\n", path.display()));
                    out.push_str(&format!("+++ b/{}\n", path.display()));
                    out.push_str(&unified_diff(&o.content, &n.content));
                    out.push('\n');
                }
                (Some(o), None) => {
                    out.push_str(&format!("--- a/{}\n", path.display()));
                    out.push_str(&format!("+++ /dev/null\n"));
                    out.push_str(&deletion_diff(&o.content));
                    out.push('\n');
                }
                (None, Some(n)) => {
                    out.push_str(&format!("--- /dev/null\n"));
                    out.push_str(&format!("+++ b/{}\n", path.display()));
                    out.push_str(&addition_diff(&n.content));
                    out.push('\n');
                }
                _ => {}
            }
        }
        out
    }
}

fn unified_diff(old: &[u8], new: &[u8]) -> String {
    let old_str = String::from_utf8_lossy(old);
    let new_str = String::from_utf8_lossy(new);
    let old_lines: Vec<&str> = old_str.lines().collect();
    let new_lines: Vec<&str> = new_str.lines().collect();
    
    let mut out = String::new();
    let mut i = 0;
    let mut j = 0;
    
    while i < old_lines.len() || j < new_lines.len() {
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            i += 1;
            j += 1;
            continue;
        }
        
        let old_start = i + 1;
        let new_start = j + 1;
        let mut old_block = Vec::new();
        let mut new_block = Vec::new();
        
        while i < old_lines.len() && j < new_lines.len() && old_lines[i] != new_lines[j] {
            if old_lines[i] != new_lines[j] {
                old_block.push(old_lines[i]);
                new_block.push(new_lines[j]);
            }
            i += 1;
            j += 1;
            if old_block.len() >= 50 { break; }
        }
        
        out.push_str(&format!("@@ -{},{} +{},{} @@\n", 
            old_start, old_block.len(), new_start, new_block.len()));
        
        for line in &old_block { out.push_str(&format!("-{}\n", line)); }
        for line in &new_block { out.push_str(&format!("+{}\n", line)); }
    }
    out
}

fn deletion_diff(content: &[u8]) -> String {
    let binding = String::from_utf8_lossy(content);
    let lines: Vec<&str> = binding.lines().collect();
    let mut out = String::new();
    out.push_str(&format!("@@ -1,{} +0,0 @@\n", lines.len()));
    for line in lines { out.push_str(&format!("-{}\n", line)); }
    out
}

fn addition_diff(content: &[u8]) -> String {
    let binding = String::from_utf8_lossy(content);
    let lines: Vec<&str> = binding.lines().collect();
    let mut out = String::new();
    out.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
    for line in lines { out.push_str(&format!("+{}\n", line)); }
    out
}

fn list_saves() -> Result<()> {
    if !Path::new(".diffpatch/master").exists() {
        eprintln!("  (no snapshots found)");
        return Ok(());
    }
    for line in BufReader::new(fs::File::open(".diffpatch/master")?).lines() {
        let line = line?;
        if let Some((name, hash)) = line.split_once(':') {
            println!("  {} [{}]", name, hash);
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("diffpatch - dynamic snapshot & patch tool");
    eprintln!();
    eprintln!("usage: diffpatch {{command}} [arguments]");
    eprintln!();
    eprintln!("commands:");
    eprintln!("  add <path>              create snapshot of file/directory");
    eprintln!("  save <name>             save current state as <name>");
    eprintln!("  list                    list all saved snapshots");
    eprintln!("  generate <hash1> <hash2> generate unified diff");
    eprintln!("  help                    show this help");
    eprintln!();
    eprintln!("examples:");
    eprintln!("  diffpatch add ./src");
    eprintln!("  diffpatch save v1.0");
    eprintln!("  diffpatch list");
    eprintln!("  diffpatch generate a1b2c3d4 e5f6g7h8 > changes.patch");
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }
    
    match args[1].as_str() {
        "add" => {
            if args.len() < 3 {
                eprintln!("error: missing path");
                eprintln!("usage: diffpatch add <path>");
                std::process::exit(1);
            }
            let start = Instant::now();
            let mut idx = Index::new();
            idx.add_path(Path::new(&args[2]))?;
            let hash = idx.compute_root_hash();
            eprintln!("ok: snapshot created");
            eprintln!("    hash: {}", hash);
            eprintln!("    time: {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);
        }
        "save" => {
            if args.len() < 3 {
                eprintln!("error: missing name");
                eprintln!("usage: diffpatch save <name>");
                std::process::exit(1);
            }
            let start = Instant::now();
            let mut idx = Index::new();
            idx.add_path(Path::new("."))?;
            let hash = idx.save(&args[2])?;
            eprintln!("ok: saved as '{}'", args[2]);
            eprintln!("    hash: {}", hash);
            eprintln!("    time: {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);
        }
        "list" => {
            list_saves()?;
        }
        "generate" => {
            if args.len() < 4 {
                eprintln!("error: missing hashes");
                eprintln!("usage: diffpatch generate <hash1> <hash2>");
                std::process::exit(1);
            }
            let start = Instant::now();
            if !Path::new(".diffpatch/master").exists() {
                eprintln!("error: no snapshots found");
                std::process::exit(1);
            }
            let master = fs::read_to_string(".diffpatch/master")?;
            let mut snaps = Vec::new();
            let mut found1 = false;
            let mut found2 = false;
            
            for line in master.lines() {
                let (name, hash) = line.split_once(':').unwrap();
                if hash == args[2] {
                    snaps.push(Index::load(name, hash)?);
                    found1 = true;
                } else if hash == args[3] {
                    snaps.push(Index::load(name, hash)?);
                    found2 = true;
                }
            }
            
            if !found1 || !found2 {
                eprintln!("error: snapshot hashes not found");
                let mark1 = if !found1 { "!" } else { " " };
                let mark2 = if !found2 { "!" } else { " " };
                eprintln!("       '{}' {}", mark1, args[2]);
                eprintln!("       '{}' {}", mark2, args[3]);
                std::process::exit(1);
            }
            
            let patch = snaps[0].diff(&snaps[1]);
            print!("{}", patch);
            eprintln!();
            eprintln!("ok: diff generated");
            eprintln!("    time: {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);
        }
        "help" | "-h" | "--help" => {
            print_usage();
        }
        _ => {
            eprintln!("error: unknown command '{}'", args[1]);
            eprintln!("try 'diffpatch help'");
            std::process::exit(1);
        }
    }
    Ok(())
}