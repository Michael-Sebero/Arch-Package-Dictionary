use std::io::Write;
use std::process::{Command, Stdio};
use std::env;
use tokio::runtime::Runtime;
use futures::future::join_all;

// ANSI color codes as constants
const BOLD: &str = "\x1B[1m";
const BLUE: &str = "\x1B[34m";
const RED: &str = "\x1B[31m";
const GREEN: &str = "\x1B[32m";
const RESET: &str = "\x1B[0m";

#[derive(Clone)]
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
    
    // Create a tokio runtime for async operations
    let rt = Runtime::new().expect("Failed to create runtime");
    
    // Add error handling for the async search
    match rt.block_on(search_packages(&search_term)) {
        Ok(results) => print_results_with_pager(&results),
        Err(e) => {
            eprintln!("{}Error:{} Failed to search packages: {}", RED, RESET, e);
            std::process::exit(1);
        }
    }
}

async fn search_packages(term: &str) -> Result<(Vec<PackageInfo>, Vec<PackageInfo>, Vec<PackageInfo>), Box<dyn std::error::Error>> {
    // Clone the term once for each async task
    let term_pacman = term.to_string();
    let term_aur = term.to_string();
    let term_flatpak = term.to_string();
    
    // Create three async tasks for concurrent execution
    let pacman_search = tokio::spawn(async move {
        search_pacman(&term_pacman)
    });
    
    let aur_search = tokio::spawn(async move {
        search_aur(&term_aur)
    });
    
    let flatpak_search = tokio::spawn(async move {
        search_flatpak(&term_flatpak)
    });

    // Wait for all searches to complete
    let results = join_all(vec![pacman_search, aur_search, flatpak_search]).await;
    
    // Unwrap the results, using empty vectors as fallback
    let mut final_results = (Vec::new(), Vec::new(), Vec::new());
    
    // Better error handling for each search
    match &results[0] {
        Ok(Ok(pacman_results)) => final_results.0 = pacman_results.to_vec(),
        Ok(Err(e)) => eprintln!("{}Warning:{} Pacman search failed: {}", RED, RESET, e),
        Err(e) => eprintln!("{}Warning:{} Pacman task failed: {}", RED, RESET, e),
    }
    
    match &results[1] {
        Ok(Ok(aur_results)) => final_results.1 = aur_results.to_vec(),
        Ok(Err(e)) => eprintln!("{}Warning:{} AUR search failed: {}", RED, RESET, e),
        Err(e) => eprintln!("{}Warning:{} AUR task failed: {}", RED, RESET, e),
    }
    
    match &results[2] {
        Ok(Ok(flatpak_results)) => final_results.2 = flatpak_results.to_vec(),
        Ok(Err(e)) => eprintln!("{}Warning:{} Flatpak search failed: {}", RED, RESET, e),
        Err(e) => eprintln!("{}Warning:{} Flatpak task failed: {}", RED, RESET, e),
    }

    Ok(final_results)
}

fn search_pacman(term: &str) -> std::io::Result<Vec<PackageInfo>> {
    let output = Command::new("pacman")
        .args(&["-Ss", term])
        .output()?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    
    let mut results = Vec::new();
    let mut lines = stdout.lines().peekable();
    
    while let Some(line) = lines.next() {
        if line.contains('/') {
            // Parse the package line which contains repo/name version
            let parts: Vec<&str> = line.splitn(2, '/').collect();
            if parts.len() == 2 {
                let name_version: Vec<&str> = parts[1].splitn(2, ' ').collect();
                if name_version.len() == 2 {
                    let name = name_version[0].trim();
                    // Extract version from the remaining part
                    let version_part = name_version[1].trim();
                    let version = if version_part.starts_with('(') && version_part.contains(')') {
                        // Extract what's inside the parentheses
                        let v_start = version_part.find('(').unwrap_or(0) + 1;
                        let v_end = version_part.find(')').unwrap_or(version_part.len());
                        if v_start < v_end {
                            version_part[v_start..v_end].to_string()
                        } else {
                            version_part.to_string()
                        }
                    } else {
                        version_part.to_string()
                    };
                    
                    // Get description from the next line if available
                    let description = if let Some(desc_line) = lines.next() {
                        if desc_line.trim().is_empty() {
                            "No description.".to_string()
                        } else {
                            desc_line.trim().to_string()
                        }
                    } else {
                        "No description.".to_string()
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

fn search_aur(term: &str) -> std::io::Result<Vec<PackageInfo>> {
    // Check if paru is installed
    if Command::new("which").arg("paru").output()?.status.success() {
        let output = Command::new("paru")
            .args(&["-Ss", "--aur", term])
            .output()?;
        
        parse_aur_output(&output.stdout)
    } else if Command::new("which").arg("yay").output()?.status.success() {
        // Try yay if paru is not available
        let output = Command::new("yay")
            .args(&["-Ss", "--aur", term])
            .output()?;
        
        parse_aur_output(&output.stdout)
    } else {
        eprintln!("{}Warning:{} No AUR helper found (tried paru and yay). AUR search disabled.", RED, RESET);
        Ok(Vec::new())
    }
}

fn parse_aur_output(stdout: &[u8]) -> std::io::Result<Vec<PackageInfo>> {
    let stdout = String::from_utf8_lossy(stdout);
    
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    
    let mut results = Vec::new();
    let mut lines = stdout.lines().peekable();
    
    while let Some(line) = lines.next() {
        if line.contains("aur/") {
            // Parse the package line which contains aur/name version
            let parts: Vec<&str> = line.splitn(2, '/').collect();
            if parts.len() == 2 {
                let name_version: Vec<&str> = parts[1].splitn(2, ' ').collect();
                if name_version.len() == 2 {
                    let name = name_version[0].trim();
                    
                    // Extract version from the remaining part
                    let version_part = name_version[1].trim();
                    let version = if version_part.starts_with('(') && version_part.contains(')') {
                        // Extract what's inside the parentheses
                        let v_start = version_part.find('(').unwrap_or(0) + 1;
                        let v_end = version_part.find(')').unwrap_or(version_part.len());
                        if v_start < v_end {
                            version_part[v_start..v_end].to_string()
                        } else {
                            version_part.to_string()
                        }
                    } else {
                        version_part.to_string()
                    };
                    
                    // Get description from the next line if available
                    let description = if let Some(desc_line) = lines.next() {
                        if desc_line.trim().is_empty() {
                            "No description.".to_string()
                        } else {
                            desc_line.trim().to_string()
                        }
                    } else {
                        "No description.".to_string()
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

fn search_flatpak(term: &str) -> std::io::Result<Vec<PackageInfo>> {
    // Check if flatpak is installed
    if !Command::new("which").arg("flatpak").output()?.status.success() {
        eprintln!("{}Warning:{} Flatpak not found. Flatpak search disabled.", RED, RESET);
        return Ok(Vec::new());
    }

    let output = Command::new("flatpak")
        .args(&["search", term])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    
    let mut results = Vec::new();
    let lines = stdout.lines().skip(1); // Skip header row
    
    for line in lines {
        if line.is_empty() {
            continue;
        }
        
        let parts: Vec<&str> = line.split('\t').collect();
        
        if parts.len() >= 3 {
            let name = parts[0].trim();
            let version = match parts.get(1) {
                Some(&v) if !v.trim().is_empty() => v.trim().to_string(),
                _ => "Unknown".to_string(),
            };
            
            let description = match parts.get(2) {
                Some(&d) if !d.trim().is_empty() => d.trim().to_string(),
                _ => "No description.".to_string(),
            };
            
            // Only add if the package matches the search term
            if name.to_lowercase().contains(&term.to_lowercase()) {
                results.push(PackageInfo {
                    name: name.to_string(),
                    version,
                    description,
                });
            }
        }
    }
    
    Ok(results)
}

fn print_results_with_pager(results: &(Vec<PackageInfo>, Vec<PackageInfo>, Vec<PackageInfo>)) {
    let (pacman, aur, flatpak) = results;
    
    let mut output = String::new();
    
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

    // Check if we should use pager based on output size and terminal height
    let use_pager = output.lines().count() > 20; // Arbitrary threshold

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
