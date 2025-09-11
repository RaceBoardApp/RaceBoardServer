use regex::Regex;

// Central adapter ID utilities
// Convention: "adapter:{type}:{instance}"
// - type: lowercase letters, digits, and dashes (e.g., gitlab, github-actions)
// - instance: letters, digits, dashes, and underscores
// - max length per segment: 64
lazy_static::lazy_static! {
    static ref TYPE_RE: Regex = Regex::new(r"^[a-z0-9-]{1,64}$").unwrap();
    static ref INSTANCE_RE: Regex = Regex::new(r"^[A-Za-z0-9_-]{1,64}$").unwrap();
}

pub fn is_adapter_id(id: &str) -> bool {
    if let Some(stripped) = id.strip_prefix("adapter:") {
        let mut parts = stripped.splitn(2, ':');
        if let (Some(t), Some(inst)) = (parts.next(), parts.next()) {
            return TYPE_RE.is_match(t) && INSTANCE_RE.is_match(inst);
        }
    }
    false
}

pub fn validate(id: &str) -> Result<(), String> {
    if is_adapter_id(id) {
        Ok(())
    } else {
        Err("invalid adapter id; expected format adapter:{type}:{instance}".to_string())
    }
}
