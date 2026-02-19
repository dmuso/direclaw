use crate::config::Settings;
use crate::queue::IncomingMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseEligibility {
    NotApplicable,
    MustReply,
    Opportunistic,
}

pub fn classify_response_eligibility(
    _settings: &Settings,
    inbound: &IncomingMessage,
) -> ResponseEligibility {
    if inbound.is_direct || is_explicit_profile_mention(inbound) {
        return ResponseEligibility::MustReply;
    }
    ResponseEligibility::Opportunistic
}

pub fn is_explicit_profile_mention(inbound: &IncomingMessage) -> bool {
    inbound.is_mentioned
}

#[cfg(test)]
mod tests {
    use super::{classify_response_eligibility, ResponseEligibility};
    use crate::config::{
        ChannelKind, ChannelProfile, ChannelProfileIdentity, Settings, SettingsOrchestrator,
        ThreadResponseMode,
    };
    use crate::queue::IncomingMessage;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn settings() -> Settings {
        Settings {
            workspaces_path: PathBuf::from("/tmp"),
            shared_workspaces: BTreeMap::new(),
            orchestrators: BTreeMap::from([(
                "main".to_string(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            )]),
            channel_profiles: BTreeMap::from([(
                "slack-main".to_string(),
                ChannelProfile {
                    channel: ChannelKind::Slack,
                    orchestrator_id: "main".to_string(),
                    identity: ChannelProfileIdentity::default(),
                    slack_app_user_id: Some("UAPP".to_string()),
                    require_mention_in_channels: Some(true),
                    thread_response_mode: ThreadResponseMode::AlwaysReply,
                },
            )]),
            monitoring: Default::default(),
            channels: BTreeMap::new(),
            auth_sync: Default::default(),
            memory: Default::default(),
        }
    }

    fn base_slack_message() -> IncomingMessage {
        IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("slack-main".to_string()),
            sender: "dana".to_string(),
            sender_id: "U1".to_string(),
            message: "hello".to_string(),
            timestamp: 1,
            message_id: "slack-slack-main-C111-200_0".to_string(),
            conversation_id: Some("C111:100.1".to_string()),
            is_direct: false,
            is_thread_reply: true,
            is_mentioned: false,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        }
    }

    #[test]
    fn classifies_dm_as_must_reply() {
        let mut inbound = base_slack_message();
        inbound.is_direct = true;
        assert_eq!(
            classify_response_eligibility(&settings(), &inbound),
            ResponseEligibility::MustReply
        );
    }

    #[test]
    fn classifies_explicit_mention_as_must_reply() {
        let mut inbound = base_slack_message();
        inbound.message = "<@UAPP> help".to_string();
        inbound.is_mentioned = true;
        assert_eq!(
            classify_response_eligibility(&settings(), &inbound),
            ResponseEligibility::MustReply
        );
    }

    #[test]
    fn classifies_non_dm_non_mention_as_opportunistic() {
        let mut cfg = settings();
        cfg.channel_profiles
            .get_mut("slack-main")
            .unwrap()
            .thread_response_mode = ThreadResponseMode::SelectiveReply;
        let inbound = base_slack_message();
        assert_eq!(
            classify_response_eligibility(&cfg, &inbound),
            ResponseEligibility::Opportunistic
        );
    }

    #[test]
    fn selective_reply_dm_is_must_reply() {
        let mut cfg = settings();
        cfg.channel_profiles
            .get_mut("slack-main")
            .expect("profile")
            .thread_response_mode = ThreadResponseMode::SelectiveReply;
        let mut inbound = base_slack_message();
        inbound.conversation_id = Some("D111:100.1".to_string());
        inbound.message_id = "slack-slack-main-D111-200_0".to_string();
        inbound.is_direct = true;
        assert_eq!(
            classify_response_eligibility(&cfg, &inbound),
            ResponseEligibility::MustReply
        );
    }

    #[test]
    fn non_direct_non_mentioned_top_level_message_is_opportunistic() {
        let cfg = settings();
        let mut inbound = base_slack_message();
        inbound.conversation_id = Some("C111:200.0".to_string());
        inbound.message_id = "slack-slack-main-C111-200_0".to_string();
        inbound.is_thread_reply = false;
        assert_eq!(
            classify_response_eligibility(&cfg, &inbound),
            ResponseEligibility::Opportunistic
        );
    }

    #[test]
    fn non_direct_non_mentioned_is_opportunistic_even_for_always_reply_mode() {
        let cfg = settings();
        let inbound = base_slack_message();
        assert_eq!(
            classify_response_eligibility(&cfg, &inbound),
            ResponseEligibility::Opportunistic
        );
    }

    #[test]
    fn selective_reply_non_slack_thread_is_opportunistic() {
        let mut cfg = settings();
        cfg.channel_profiles.insert(
            "local-main".to_string(),
            ChannelProfile {
                channel: ChannelKind::Local,
                orchestrator_id: "main".to_string(),
                identity: ChannelProfileIdentity::default(),
                slack_app_user_id: None,
                require_mention_in_channels: None,
                thread_response_mode: ThreadResponseMode::SelectiveReply,
            },
        );
        let inbound = IncomingMessage {
            channel: "local".to_string(),
            channel_profile_id: Some("local-main".to_string()),
            sender: "cli".to_string(),
            sender_id: "cli".to_string(),
            message: "background chatter".to_string(),
            timestamp: 1,
            message_id: "msg-local-opportunistic".to_string(),
            conversation_id: Some("chat-1".to_string()),
            is_direct: false,
            is_thread_reply: true,
            is_mentioned: false,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        };
        assert_eq!(
            classify_response_eligibility(&cfg, &inbound),
            ResponseEligibility::Opportunistic
        );
    }

    #[test]
    fn explicit_non_slack_profile_mention_is_must_reply() {
        let mut cfg = settings();
        cfg.channel_profiles.insert(
            "local-main".to_string(),
            ChannelProfile {
                channel: ChannelKind::Local,
                orchestrator_id: "main".to_string(),
                identity: ChannelProfileIdentity::default(),
                slack_app_user_id: None,
                require_mention_in_channels: None,
                thread_response_mode: ThreadResponseMode::SelectiveReply,
            },
        );
        let inbound = IncomingMessage {
            channel: "local".to_string(),
            channel_profile_id: Some("local-main".to_string()),
            sender: "cli".to_string(),
            sender_id: "cli".to_string(),
            message: "agent please respond".to_string(),
            timestamp: 1,
            message_id: "msg-local-mention".to_string(),
            conversation_id: Some("chat-1".to_string()),
            is_direct: false,
            is_thread_reply: true,
            is_mentioned: true,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        };
        assert_eq!(
            classify_response_eligibility(&cfg, &inbound),
            ResponseEligibility::MustReply
        );
    }
}
