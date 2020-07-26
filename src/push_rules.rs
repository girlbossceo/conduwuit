use js_int::uint;
use ruma::{
    UserId,
    push::{
        Action, ConditionalPushRule, ConditionalPushRuleInit, PatternedPushRule,
        PatternedPushRuleInit, PushCondition, RoomMemberCountIs, Ruleset, Tweak,
    },
};

pub fn default_pushrules(user_id: &UserId) -> Ruleset {
    let mut rules = Ruleset::default();
    rules.content = vec![contains_user_name_rule(&user_id)];
    rules.override_ = vec![
        master_rule(),
        suppress_notices_rule(),
        invite_for_me_rule(),
        member_event_rule(),
        contains_display_name_rule(),
        tombstone_rule(),
        roomnotif_rule(),
    ];
    rules.underride = vec![
        call_rule(),
        encrypted_room_one_to_one_rule(),
        room_one_to_one_rule(),
        message_rule(),
        encrypted_rule(),
    ];
    rules
}

pub fn master_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![Action::DontNotify],
        default: true,
        enabled: false,
        rule_id: ".m.rule.master".to_owned(),
        conditions: vec![],
    }
    .into()
}

pub fn suppress_notices_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![Action::DontNotify],
        default: true,
        enabled: true,
        rule_id: ".m.rule.suppress_notices".to_owned(),
        conditions: vec![PushCondition::EventMatch {
            key: "content.msgtype".to_owned(),
            pattern: "m.notice".to_owned(),
        }],
    }
    .into()
}

pub fn invite_for_me_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![
            Action::Notify,
            Action::SetTweak(Tweak::Sound("default".to_owned())),
            Action::SetTweak(Tweak::Highlight(false)),
        ],
        default: true,
        enabled: true,
        rule_id: ".m.rule.invite_for_me".to_owned(),
        conditions: vec![PushCondition::EventMatch {
            key: "content.membership".to_owned(),
            pattern: "m.invite".to_owned(),
        }],
    }
    .into()
}

pub fn member_event_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![Action::DontNotify],
        default: true,
        enabled: true,
        rule_id: ".m.rule.member_event".to_owned(),
        conditions: vec![PushCondition::EventMatch {
            key: "content.membership".to_owned(),
            pattern: "type".to_owned(),
        }],
    }
    .into()
}

pub fn contains_display_name_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![
            Action::Notify,
            Action::SetTweak(Tweak::Sound("default".to_owned())),
            Action::SetTweak(Tweak::Highlight(true)),
        ],
        default: true,
        enabled: true,
        rule_id: ".m.rule.contains_display_name".to_owned(),
        conditions: vec![PushCondition::ContainsDisplayName],
    }
    .into()
}

pub fn tombstone_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![Action::Notify, Action::SetTweak(Tweak::Highlight(true))],
        default: true,
        enabled: true,
        rule_id: ".m.rule.tombstone".to_owned(),
        conditions: vec![
            PushCondition::EventMatch {
                key: "type".to_owned(),
                pattern: "m.room.tombstone".to_owned(),
            },
            PushCondition::EventMatch {
                key: "state_key".to_owned(),
                pattern: "".to_owned(),
            },
        ],
    }
    .into()
}

pub fn roomnotif_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![Action::Notify, Action::SetTweak(Tweak::Highlight(true))],
        default: true,
        enabled: true,
        rule_id: ".m.rule.roomnotif".to_owned(),
        conditions: vec![
            PushCondition::EventMatch {
                key: "content.body".to_owned(),
                pattern: "@room".to_owned(),
            },
            PushCondition::SenderNotificationPermission {
                key: "room".to_owned(),
            },
        ],
    }
    .into()
}

pub fn contains_user_name_rule(user_id: &UserId) -> PatternedPushRule {
    PatternedPushRuleInit {
        actions: vec![
            Action::Notify,
            Action::SetTweak(Tweak::Sound("default".to_owned())),
            Action::SetTweak(Tweak::Highlight(true)),
        ],
        default: true,
        enabled: true,
        rule_id: ".m.rule.contains_user_name".to_owned(),
        pattern: user_id.localpart().to_owned(),
    }
    .into()
}

pub fn call_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![
            Action::Notify,
            Action::SetTweak(Tweak::Sound("ring".to_owned())),
            Action::SetTweak(Tweak::Highlight(false)),
        ],
        default: true,
        enabled: true,
        rule_id: ".m.rule.call".to_owned(),
        conditions: vec![PushCondition::EventMatch {
            key: "type".to_owned(),
            pattern: "m.call.invite".to_owned(),
        }],
    }
    .into()
}

pub fn encrypted_room_one_to_one_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![
            Action::Notify,
            Action::SetTweak(Tweak::Sound("default".to_owned())),
            Action::SetTweak(Tweak::Highlight(false)),
        ],
        default: true,
        enabled: true,
        rule_id: ".m.rule.encrypted_room_one_to_one".to_owned(),
        conditions: vec![
            PushCondition::RoomMemberCount {
                is: RoomMemberCountIs::from(uint!(2)..),
            },
            PushCondition::EventMatch {
                key: "type".to_owned(),
                pattern: "m.room.encrypted".to_owned(),
            },
        ],
    }
    .into()
}

pub fn room_one_to_one_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![
            Action::Notify,
            Action::SetTweak(Tweak::Sound("default".to_owned())),
            Action::SetTweak(Tweak::Highlight(false)),
        ],
        default: true,
        enabled: true,
        rule_id: ".m.rule.room_one_to_one".to_owned(),
        conditions: vec![
            PushCondition::RoomMemberCount {
                is: RoomMemberCountIs::from(uint!(2)..),
            },
            PushCondition::EventMatch {
                key: "type".to_owned(),
                pattern: "m.room.message".to_owned(),
            },
        ],
    }
    .into()
}

pub fn message_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![Action::Notify, Action::SetTweak(Tweak::Highlight(false))],
        default: true,
        enabled: true,
        rule_id: ".m.rule.message".to_owned(),
        conditions: vec![PushCondition::EventMatch {
            key: "type".to_owned(),
            pattern: "m.room.message".to_owned(),
        }],
    }
    .into()
}

pub fn encrypted_rule() -> ConditionalPushRule {
    ConditionalPushRuleInit {
        actions: vec![Action::Notify, Action::SetTweak(Tweak::Highlight(false))],
        default: true,
        enabled: true,
        rule_id: ".m.rule.encrypted".to_owned(),
        conditions: vec![PushCondition::EventMatch {
            key: "type".to_owned(),
            pattern: "m.room.encrypted".to_owned(),
        }],
    }
    .into()
}
