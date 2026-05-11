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
    eprintln!("usage: diffpatch <command> [arguments]");
    eprintln!("       diffpatch <hash1> <hash2> [-o <file>]");
    eprintln!();
    eprintln!("commands:");
    eprintln!("  add <path> [--skip-save]   snapshot a file/dir (saves if --skip-save absent)");
    eprintln!("  save <name>                save current state as <name>");
    eprintln!("  list                       list all snapshots");
    eprintln!("  help                       show this help");
    eprintln!();
    eprintln!("examples:");
    eprintln!("  diffpatch add ./src                # saves snapshot with hash as name");
    eprintln!("  diffpatch add ./src --skip-save    # only show hash, no save");
    eprintln!("  diffpatch save v1.0");
    eprintln!("  diffpatch list");
    eprintln!("  diffpatch a1b2c3d4 e5f6g7h8 > out.patch");
    eprintln!("  diffpatch a1b2c3d4 e5f6g7h8 -o out.patch");
}

fn is_valid_hash(hash: &str, master_path: &Path) -> Result<bool> {
    if !master_path.exists() {
        return Ok(false);
    }
    for line in BufReader::new(fs::File::open(master_path)?).lines() {
        let line = line?;
        if let Some((_, h)) = line.split_once(':') {
            if h == hash {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn load_snapshot_by_hash(hash: &str, master_path: &Path) -> Result<Option<Index>> {
    for line in BufReader::new(fs::File::open(master_path)?).lines() {
        let line = line?;
        if let Some((name, h)) = line.split_once(':') {
            if h == hash {
                return Ok(Some(Index::load(name, hash)?));
            }
        }
    }
    Ok(None)
}

fn run_generate(hash1: &str, hash2: &str, out_file: Option<&str>) -> Result<()> {
    let start = Instant::now();
    let master_path = Path::new(".diffpatch/master");
    if !master_path.exists() {
        eprintln!("error: no snapshots found");
        std::process::exit(1);
    }
    
    let snap1 = match load_snapshot_by_hash(hash1, master_path)? {
        Some(s) => s,
        None => {
            eprintln!("error: hash '{}' not found", hash1);
            std::process::exit(1);
        }
    };
    let snap2 = match load_snapshot_by_hash(hash2, master_path)? {
        Some(s) => s,
        None => {
            eprintln!("error: hash '{}' not found", hash2);
            std::process::exit(1);
        }
    };
    
    let patch = snap1.diff(&snap2);
    match out_file {
        Some(filename) => {
            fs::write(filename, patch)?;
            eprintln!("ok: diff written to {}", filename);
        }
        None => {
            print!("{}", patch);
        }
    }
    eprintln!("    time: {:.2}ms", start.elapsed().as_secs_f64() * 1000.0);
    Ok(())
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
                eprintln!("usage: diffpatch add <path> [--skip-save]");
                std::process::exit(1);
            }
            let mut skip_save = false;
            let mut path = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--skip-save" {
                    skip_save = true;
                } else if path.is_none() {
                    path = Some(&args[i]);
                } else {
                    eprintln!("error: unexpected argument '{}'", args[i]);
                    std::process::exit(1);
                }
                i += 1;
            }
            let path = path.expect("path missing");
            let start = Instant::now();
            let mut idx = Index::new();
            idx.add_path(Path::new(path))?;
            let hash = idx.compute_root_hash();
            
            if skip_save {
                eprintln!("ok: snapshot created (not saved)");
                eprintln!("    hash: {}", hash);
            } else {
                idx.save(&hash)?;
                eprintln!("ok: snapshot saved as '{}'", hash);
                eprintln!("    hash: {}", hash);
            }
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
        "help" | "-h" | "--help" => {
            print_usage();
        }
        _ => {
            if args.len() >= 3 {
                let master_path = Path::new(".diffpatch/master");
                if master_path.exists() {
                    let hash1 = &args[1];
                    let hash2 = &args[2];
                    if is_valid_hash(hash1, master_path)? && is_valid_hash(hash2, master_path)? {
                        let mut out_file = None;
                        let mut i = 3;
                        while i < args.len() {
                            if args[i] == "-o" && i + 1 < args.len() {
                                out_file = Some(&args[i+1]);
                                i += 2;
                            } else {
                                eprintln!("error: unexpected argument '{}'", args[i]);
                                std::process::exit(1);
                            }
                        }
                        run_generate(hash1, hash2, out_file)?;
                        return Ok(());
                    }
                }
            }
            eprintln!("error: unknown command or invalid hash pair '{}'", args[1]);
            eprintln!("try 'diffpatch help'");
            std::process::exit(1);
        }
    }
    Ok(())
}