use crate::app::command_support::{load_settings, save_settings};
use crate::config::{ChannelKind, ChannelProfile};

pub fn cmd_channel_profile(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(
            "usage: channel-profile <list|add|show|remove|set-orchestrator> ...".to_string(),
        );
    }

    match args[0].as_str() {
        "list" => {
            let settings = load_settings()?;
            Ok(settings
                .channel_profiles
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "add" => {
            if args.len() < 4 {
                return Err("usage: channel-profile add <channel_profile_id> <channel> <orchestrator_id> [--slack-app-user-id <id>] [--require-mention-in-channels <bool>]".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            let channel = ChannelKind::parse(&args[2])?;
            let orchestrator_id = args[3].clone();
            if !settings.orchestrators.contains_key(&orchestrator_id) {
                return Err(format!("unknown orchestrator `{orchestrator_id}`"));
            }
            if settings.channel_profiles.contains_key(&id) {
                return Err(format!("channel profile `{id}` already exists"));
            }

            let mut slack_app_user_id = None;
            let mut require_mention = None;
            let mut i = 4usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--slack-app-user-id" => {
                        if i + 1 >= args.len() {
                            return Err("missing value for --slack-app-user-id".to_string());
                        }
                        slack_app_user_id = Some(args[i + 1].clone());
                        i += 2;
                    }
                    "--require-mention-in-channels" => {
                        if i + 1 >= args.len() {
                            return Err(
                                "missing value for --require-mention-in-channels".to_string()
                            );
                        }
                        require_mention = Some(parse_bool(&args[i + 1])?);
                        i += 2;
                    }
                    other => return Err(format!("unknown option `{other}`")),
                }
            }

            settings.channel_profiles.insert(
                id.clone(),
                ChannelProfile {
                    channel,
                    orchestrator_id,
                    slack_app_user_id,
                    require_mention_in_channels: require_mention,
                },
            );
            save_settings(&settings)?;
            Ok(format!("channel profile added\nid={id}"))
        }
        "show" => {
            if args.len() != 2 {
                return Err("usage: channel-profile show <channel_profile_id>".to_string());
            }
            let settings = load_settings()?;
            let profile = settings
                .channel_profiles
                .get(&args[1])
                .ok_or_else(|| format!("unknown channel profile `{}`", args[1]))?;
            let mention_policy = profile
                .require_mention_in_channels
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_string());
            Ok(format!(
                "id={}\nchannel={}\norchestrator_id={}\nslack_app_user_id={}\nrequire_mention_in_channels={}",
                args[1],
                profile.channel,
                profile.orchestrator_id,
                profile
                    .slack_app_user_id
                    .clone()
                    .unwrap_or_else(|| "n/a".to_string()),
                mention_policy
            ))
        }
        "remove" => {
            if args.len() != 2 {
                return Err("usage: channel-profile remove <channel_profile_id>".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            if settings.channel_profiles.remove(&id).is_none() {
                return Err(format!("unknown channel profile `{id}`"));
            }
            save_settings(&settings)?;
            Ok(format!("channel profile removed\nid={id}"))
        }
        "set-orchestrator" => {
            if args.len() != 3 {
                return Err("usage: channel-profile set-orchestrator <channel_profile_id> <orchestrator_id>".to_string());
            }
            let mut settings = load_settings()?;
            let profile_id = &args[1];
            let orchestrator_id = args[2].clone();
            if !settings.orchestrators.contains_key(&orchestrator_id) {
                return Err(format!("unknown orchestrator `{orchestrator_id}`"));
            }
            let profile = settings
                .channel_profiles
                .get_mut(profile_id)
                .ok_or_else(|| format!("unknown channel profile `{profile_id}`"))?;
            profile.orchestrator_id = orchestrator_id.clone();
            save_settings(&settings)?;
            Ok(format!(
                "channel profile updated\nid={}\norchestrator_id={}",
                profile_id, orchestrator_id
            ))
        }
        other => Err(format!("unknown channel-profile subcommand `{other}`")),
    }
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected boolean true|false, got `{raw}`")),
    }
}
