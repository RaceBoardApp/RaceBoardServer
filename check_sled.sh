#!/bin/bash

# Quick script to check what's in sled database
echo "Checking sled database structure..."

cat > /tmp/check_sled.rs << 'EOF'
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".raceboard/eta_history.db");
    
    println!("Opening database at: {:?}", db_path);
    let db = sled::open(&db_path)?;
    
    println!("\nAvailable trees:");
    for name in db.tree_names() {
        let name_str = String::from_utf8_lossy(&name);
        println!("  - {}", name_str);
        
        let tree = db.open_tree(&name)?;
        let count = tree.len();
        println!("    Items: {}", count);
        
        // Show first few keys
        let mut shown = 0;
        for item in tree.iter() {
            if shown >= 3 { break; }
            let (key, _) = item?;
            let key_str = String::from_utf8_lossy(&key);
            println!("      Sample key: {}", key_str);
            shown += 1;
        }
    }
    
    // Check specific trees
    println!("\n--- Checking races tree ---");
    let races_tree = db.open_tree("races")?;
    println!("Total items in races tree: {}", races_tree.len());
    
    // Check time index
    println!("\n--- Checking races_by_time tree ---");
    let time_tree = db.open_tree("races_by_time")?;
    println!("Total items in races_by_time tree: {}", time_tree.len());
    
    // Sample some races
    println!("\n--- Sampling race IDs ---");
    let mut count_by_prefix = std::collections::HashMap::new();
    for item in races_tree.iter().take(100) {
        let (key, _) = item?;
        let key_str = String::from_utf8_lossy(&key);
        if let Some(prefix) = key_str.split(':').next() {
            *count_by_prefix.entry(prefix.to_string()).or_insert(0) += 1;
        }
    }
    
    println!("Race ID prefixes (first 100):");
    for (prefix, count) in count_by_prefix {
        println!("  {}: {}", prefix, count);
    }
    
    Ok(())
}
EOF

rustc /tmp/check_sled.rs -o /tmp/check_sled --edition 2021 --extern sled=/Users/user/RustroverProjects/RaceboardServer/target/debug/deps/libsled*.rlib --extern dirs=/Users/user/RustroverProjects/RaceboardServer/target/debug/deps/libdirs*.rlib -L /Users/user/RustroverProjects/RaceboardServer/target/debug/deps 2>/dev/null

if [ -f /tmp/check_sled ]; then
    /tmp/check_sled
else
    echo "Failed to compile check_sled"
fi