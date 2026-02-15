use crate::commands::{ensure_runtime_root, load_preferences, save_preferences};

pub fn cmd_provider(args: &[String]) -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let mut prefs = load_preferences(&paths)?;

    if args.is_empty() {
        return Ok(format!(
            "provider={}\nmodel={}",
            prefs.provider.unwrap_or_else(|| "none".to_string()),
            prefs.model.unwrap_or_else(|| "none".to_string())
        ));
    }

    let provider = args[0].clone();
    if provider != "anthropic" && provider != "openai" {
        return Err("provider must be one of: anthropic, openai".to_string());
    }

    prefs.provider = Some(provider.clone());
    if args.len() >= 3 && args[1] == "--model" {
        prefs.model = Some(args[2].clone());
    }
    save_preferences(&paths, &prefs)?;

    Ok(format!(
        "provider={}\nmodel={}",
        provider,
        prefs.model.unwrap_or_else(|| "none".to_string())
    ))
}

pub fn cmd_model(args: &[String]) -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let mut prefs = load_preferences(&paths)?;

    if args.is_empty() {
        return Ok(format!(
            "model={}",
            prefs.model.unwrap_or_else(|| "none".to_string())
        ));
    }

    prefs.model = Some(args[0].clone());
    save_preferences(&paths, &prefs)?;
    Ok(format!("model={}", args[0]))
}
