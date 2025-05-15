use std::io::Write;
use std::process::{Command, Stdio};
use std::env;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

// ANSI color codes as constants
const BOLD: &str = "\x1B[1m";
const BLUE: &str = "\x1B[34m";
const RED: &str = "\x1B[31m";
const GREEN: &str = "\x1B[32m";
const RESET: &str = "\x1B[0m";

#[derive(Clone, Debug)]
struct PackageInfo {
    name: String,
    version: String,
    description: String,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        eprintln!("{}Usage:{} pd <search-term>", BOLD, RESET);
        std::process::exit(1);
    }

    let search_term = args[1..].join(" ");
    
    // Create a tokio runtime with multi-threaded executor
    let rt = Runtime::new()
        .expect("Failed to create runtime");
    
    // Execute search with better error handling
    match rt.block_on(search_packages(&search_term)) {
        Ok(results) => {
            print_results_with_pager(&results);
        },
        Err(e) => {
            eprintln!("{}Error:{} Failed to search packages: {}", RED, RESET, e);
            std::process::exit(1);
        }
    }
}

async fn search_packages(term: &str) -> Result<(Vec<PackageInfo>, Vec<PackageInfo>, Vec<PackageInfo>), Box<dyn std::error::Error>> {
    // Use a shared string to avoid cloning for each search function
    let term = Arc::new(term.to_string());
    
    // Set up channels for returning results asynchronously
    let (pacman_tx, mut pacman_rx) = mpsc::channel(1);
    let (aur_tx, mut aur_rx) = mpsc::channel(1);
    let (flatpak_tx, mut flatpak_rx) = mpsc::channel(1);
    
    // Clone Arc references for each task
    let term_pacman = Arc::clone(&term);
    let term_aur = Arc::clone(&term);
    let term_flatpak = Arc::clone(&term);
    
    // Spawn tasks with proper error handling
    tokio::spawn(async move {
        let result = search_pacman(&term_pacman).await;
        let _ = pacman_tx.send(result).await;
    });
    
    tokio::spawn(async move {
        let result = search_aur(&term_aur).await;
        let _ = aur_tx.send(result).await;
    });
    
    tokio::spawn(async move {
        let result = search_flatpak(&term_flatpak).await;
        let _ = flatpak_tx.send(result).await;
    });

    // Collect results with timeout
    let pacman_results = match tokio::time::timeout(std::time::Duration::from_secs(5), pacman_rx.recv()).await {
        Ok(Some(Ok(results))) => results,
        Ok(Some(Err(e))) => {
            eprintln!("{}Warning:{} Pacman search failed: {}", RED, RESET, e);
            Vec::new()
        },
        Ok(None) => {
            eprintln!("{}Warning:{} Pacman search channel closed unexpectedly", RED, RESET);
            Vec::new()
        },
        Err(_) => {
            eprintln!("{}Warning:{} Pacman search timed out", RED, RESET);
            Vec::new()
        },
    };
    
    let aur_results = match tokio::time::timeout(std::time::Duration::from_secs(5), aur_rx.recv()).await {
        Ok(Some(Ok(results))) => results,
        Ok(Some(Err(e))) => {
            eprintln!("{}Warning:{} AUR search failed: {}", RED, RESET, e);
            Vec::new()
        },
        Ok(None) => {
            eprintln!("{}Warning:{} AUR search channel closed unexpectedly", RED, RESET);
            Vec::new()
        },
        Err(_) => {
            eprintln!("{}Warning:{} AUR search timed out", RED, RESET);
            Vec::new()
        },
    };
    
    let flatpak_results = match tokio::time::timeout(std::time::Duration::from_secs(5), flatpak_rx.recv()).await {
        Ok(Some(Ok(results))) => results,
        Ok(Some(Err(e))) => {
            eprintln!("{}Warning:{} Flatpak search failed: {}", RED, RESET, e);
            Vec::new()
        },
        Ok(None) => {
            eprintln!("{}Warning:{} Flatpak search channel closed unexpectedly", RED, RESET);
            Vec::new()
        },
        Err(_) => {
            eprintln!("{}Warning:{} Flatpak search timed out", RED, RESET);
            Vec::new()
        },
    };

    Ok((pacman_results, aur_results, flatpak_results))
}

async fn search_pacman(term: &str) -> std::io::Result<Vec<PackageInfo>> {
    // Use tokio process for async execution
    let output = tokio::process::Command::new("pacman")
        .args(&["-Ss", term])
        .output()
        .await?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    
    // Pre-allocate with approximate capacity
    let mut results = Vec::with_capacity(stdout.lines().count() / 2);
    let mut lines = stdout.lines().peekable();
    
    while let Some(line) = lines.next() {
        if line.contains('/') {
            // Parse the package line which contains repo/name version
            let parts: Vec<&str> = line.splitn(2, '/').collect();
            if parts.len() == 2 {
                let name_version: Vec<&str> = parts[1].splitn(2, ' ').collect();
                if name_version.len() == 2 {
                    let name = name_version[0].trim();
                    // Extract version from the remaining part more efficiently
                    let version_part = name_version[1].trim();
                    let version = if let (Some(start), Some(end)) = (version_part.find('('), version_part.find(')')) {
                        if start < end && start + 1 < version_part.len() {
                            version_part[start+1..end].to_string()
                        } else {
                            version_part.to_string()
                        }
                    } else {
                        version_part.to_string()
                    };
                    
                    // Get description from the next line if available
                    let description = match lines.next() {
                        Some(desc_line) if !desc_line.trim().is_empty() => desc_line.trim().to_string(),
                        _ => "No description.".to_string(),
                    };
                    
                    results.push(PackageInfo {
                        name: name.to_string(),
                        version,
                        description,
                    });
                }
            }
        }
    }
    
    Ok(results)
}

async fn search_aur(term: &str) -> std::io::Result<Vec<PackageInfo>> {
    // Check if paru or yay is installed
    let paru_available = tokio::process::Command::new("which")
        .arg("paru")
        .output()
        .await?
        .status
        .success();

    let output = if paru_available {
        tokio::process::Command::new("paru")
            .args(&["-Ss", "--aur", term])
            .output()
            .await?
    } else {
        // Try yay if paru is not available
        let yay_available = tokio::process::Command::new("which")
            .arg("yay")
            .output()
            .await?
            .status
            .success();
            
        if yay_available {
            tokio::process::Command::new("yay")
                .args(&["-Ss", "--aur", term])
                .output()
                .await?
        } else {
            eprintln!("{}Warning:{} No AUR helper found (tried paru and yay). AUR search disabled.", RED, RESET);
            return Ok(Vec::new());
        }
    };
    
    parse_aur_output(&output.stdout)
}

fn parse_aur_output(stdout: &[u8]) -> std::io::Result<Vec<PackageInfo>> {
    let stdout = String::from_utf8_lossy(stdout);
    
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    
    // Pre-allocate with approximate capacity
    let mut results = Vec::with_capacity(stdout.lines().count() / 2);
    let mut lines = stdout.lines().peekable();
    
    while let Some(line) = lines.next() {
        if line.contains("aur/") {
            // Parse the package line which contains aur/name version
            let parts: Vec<&str> = line.splitn(2, '/').collect();
            if parts.len() == 2 {
                let name_version: Vec<&str> = parts[1].splitn(2, ' ').collect();
                if name_version.len() == 2 {
                    let name = name_version[0].trim();
                    
                    // Extract version from the remaining part more efficiently
                    let version_part = name_version[1].trim();
                    let version = if let (Some(start), Some(end)) = (version_part.find('('), version_part.find(')')) {
                        if start < end && start + 1 < version_part.len() {
                            version_part[start+1..end].to_string()
                        } else {
                            version_part.to_string()
                        }
                    } else {
                        version_part.to_string()
                    };
                    
                    // Get description from the next line if available
                    let description = match lines.next() {
                        Some(desc_line) if !desc_line.trim().is_empty() => desc_line.trim().to_string(),
                        _ => "No description.".to_string(),
                    };
                    
                    results.push(PackageInfo {
                        name: name.to_string(),
                        version,
                        description,
                    });
                }
            }
        }
    }
    
    Ok(results)
}

async fn search_flatpak(term: &str) -> std::io::Result<Vec<PackageInfo>> {
    // Check if flatpak is installed
    let flatpak_available = tokio::process::Command::new("which")
        .arg("flatpak")
        .output()
        .await?
        .status
        .success();
        
    if !flatpak_available {
        eprintln!("{}Warning:{} Flatpak not found. Flatpak search disabled.", RED, RESET);
        return Ok(Vec::new());
    }

    // Run flatpak search with --columns to improve parsing efficiency
    let output = tokio::process::Command::new("flatpak")
        .args(&["search", "--columns=name,application,version,description", term])
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    
    // Pre-allocate with approximate capacity
    let mut results = Vec::with_capacity(stdout.lines().count());
    
    // Convert term to lowercase once for case-insensitive comparison
    let term_lower = term.to_lowercase();
    
    for line in stdout.lines().skip(1) { // Skip header row
        if line.is_empty() {
            continue;
        }
        
        let parts: Vec<&str> = line.split('\t').collect();
        
        if parts.len() >= 4 {
            let name = parts[0].trim();
            
            // Only process further if the name matches (optimization)
            if !name.to_lowercase().contains(&term_lower) {
                continue;
            }
            
            let application_id = parts[1].trim();
            let version = match parts.get(2) {
                Some(&v) if !v.trim().is_empty() => v.trim().to_string(),
                _ => "Unknown".to_string(),
            };
            
            let description = match parts.get(3) {
                Some(&d) if !d.trim().is_empty() => d.trim().to_string(),
                _ => "No description.".to_string(),
            };
            
            results.push(PackageInfo {
                name: format!("{} ({})", name, application_id),
                version,
                description,
            });
        }
    }
    
    Ok(results)
}

fn print_results_with_pager(results: &(Vec<PackageInfo>, Vec<PackageInfo>, Vec<PackageInfo>)) {
    let (pacman, aur, flatpak) = results;
    
    // Pre-allocate string buffer with approximate capacity
    let estimated_size = (pacman.len() + aur.len() + flatpak.len()) * 150;  // ~150 chars per package
    let mut output = String::with_capacity(estimated_size);
    
    fn format_package_count(count: usize) -> String {
        if count == 1 {
            format!("1 package")
        } else {
            format!("{} packages", count)
        }
    }
    
    // Summary of results
    output.push_str(&format!("{}Pacman:{} {} | {}AUR:{} {} | {}Flatpak:{} {}\n\n",
        BOLD, RESET, format_package_count(pacman.len()),
        BOLD, RESET, format_package_count(aur.len()),
        BOLD, RESET, format_package_count(flatpak.len())
    ));

    fn print_category_results(output: &mut String, category_name: &str, results: &[PackageInfo], color: &str) {
        if !results.is_empty() {
            output.push_str(&format!("{}{} Results:{}\n", BOLD, category_name, RESET));
            output.push_str(&format!("{}\n", "=".repeat(category_name.len() + 9)));
            for package in results {
                output.push_str(&format!("{}{}{}{}\n", BOLD, color, package.name, RESET));
                output.push_str(&format!("  {}\n", package.description));
                output.push_str(&format!("  {}Version:{} {}\n\n", BOLD, RESET, package.version));
            }
        }
    }

    print_category_results(&mut output, "Pacman", pacman, BLUE);
    print_category_results(&mut output, "AUR", aur, RED);
    print_category_results(&mut output, "Flatpak", flatpak, GREEN);

    // Get terminal height for better pager decisioning
    let term_height = match get_terminal_height() {
        Some(h) => h,
        None => 24, // Default fallback
    };

    // Check if we should use pager based on output size and terminal height
    let output_lines = output.lines().count();
    let use_pager = output_lines > term_height - 2;

    if use_pager {
        // Check if 'less' is available
        if Command::new("which").arg("less").output().map(|o| o.status.success()).unwrap_or(false) {
            let mut pager = Command::new("less")
                .args(&["-R", "+Gg"]) // Raw control chars, start at top
                .stdin(Stdio::piped())
                .spawn()
                .expect("Failed to start pager");

            if let Some(mut pager_stdin) = pager.stdin.take() {
                pager_stdin.write_all(output.as_bytes()).expect("Failed to write to pager");
            }

            pager.wait().expect("Pager process wasn't running");
        } else {
            // Fallback if 'less' is not available
            println!("{}", output);
        }
    } else {
        // Print directly for small outputs
        println!("{}", output);
    }
}

fn get_terminal_height() -> Option<usize> {
    // Try to get terminal size using stty
    let output = Command::new("stty")
        .args(&["size"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
        
    let size = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = size.split_whitespace().collect();
    
    if parts.len() >= 1 {
        parts[0].parse::<usize>().ok()
    } else {
        None
    }
}
