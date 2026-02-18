use direclaw::app::schedule_parsing::{parse_schedule_create_tail_args, ScheduleCreateTailArgs};
use direclaw::orchestration::scheduler::MisfirePolicy;
use serde_json::json;

#[test]
fn schedule_parsing_module_supports_named_flags_without_target_ref_positional() {
    let parsed = parse_schedule_create_tail_args(&[
        "--misfire-policy".to_string(),
        "skip_missed".to_string(),
        "--allow-overlap".to_string(),
        "true".to_string(),
    ])
    .expect("parse flags");

    assert_eq!(parsed.misfire_policy, MisfirePolicy::SkipMissed);
    assert!(parsed.allow_overlap);
    assert!(parsed.target_ref.is_none());
}

#[test]
fn schedule_parsing_module_parses_target_ref_named_flag() {
    let parsed = parse_schedule_create_tail_args(&[
        "--target-ref".to_string(),
        json!({
            "channel": "slack",
            "channelProfileId": "slack_main",
            "channelId": "C123",
            "postingMode": "channel_post"
        })
        .to_string(),
    ])
    .expect("parse target ref");

    assert_eq!(
        parsed,
        ScheduleCreateTailArgs {
            target_ref: Some(json!({
                "channel": "slack",
                "channelProfileId": "slack_main",
                "channelId": "C123",
                "postingMode": "channel_post"
            })),
            misfire_policy: MisfirePolicy::FireOnceOnRecovery,
            allow_overlap: false,
        }
    );
}
