use crate::commands;

pub fn run(args: Vec<String>) -> Result<String, String> {
    commands::run_cli(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_adapter_delegates_to_shared_commands_engine() {
        let args = vec!["unknown-command".to_string()];
        assert_eq!(run(args.clone()), commands::run_cli(args));
    }
}
