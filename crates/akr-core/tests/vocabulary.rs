//! Exit criterion 1: every kind, class, state, relation and rule in
//! `spec/tables/vocabulary.json` has a construct in code, and every construct in code is
//! in the JSON. The JSON is authoritative for names; a disagreement means the code is
//! wrong.

use akr_core::model::{Cardinality, Class, ContentSlot, Domain, Kind, Range, Relation, State};
use akr_core::validate::RULES;
use serde_json::Value;
use std::collections::BTreeSet;

fn vocabulary() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spec/tables/vocabulary.json"
    );
    let text = std::fs::read_to_string(path).expect("spec/tables/vocabulary.json is readable");
    serde_json::from_str(&text).expect("vocabulary.json is valid JSON")
}

fn names(value: &Value) -> BTreeSet<String> {
    value.as_object().expect("object").keys().cloned().collect()
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string").to_owned())
        .collect()
}

#[test]
fn every_kind_is_in_code_and_in_the_json() {
    let v = vocabulary();
    let json: BTreeSet<String> = names(&v["kinds"]);
    let code: BTreeSet<String> = Kind::ALL.iter().map(|k| k.name().to_owned()).collect();
    assert_eq!(json, code, "kind sets differ");
    assert_eq!(
        code.len(),
        13,
        "D-001 fixed twelve kinds; D-027 added papercut"
    );
}

#[test]
fn every_kind_has_the_json_class() {
    let v = vocabulary();
    for kind in Kind::ALL {
        let expected = v["kinds"][kind.name()]["class"].as_str().expect("class");
        assert_eq!(kind.class().name(), expected, "class of {kind}");
    }
}

#[test]
fn class_membership_scope_and_topic_match() {
    let v = vocabulary();
    for class in Class::ALL {
        let entry = &v["classes"][class.name()];
        let json = strings(&entry["kinds"]);
        let code: BTreeSet<String> = class.kinds().iter().map(|k| k.name().to_owned()).collect();
        assert_eq!(json, code, "kinds of class {class}");
        assert_eq!(
            class.scope_required(),
            entry["scope_required"].as_bool().expect("bool"),
            "scope_required for {class}"
        );
        assert_eq!(
            class.topic_allowed(),
            entry["topic_allowed"].as_bool().expect("bool"),
            "topic_allowed for {class}"
        );
    }
}

#[test]
fn every_content_slot_matches_the_json_in_name_order_and_requiredness() {
    let v = vocabulary();
    for kind in Kind::ALL {
        let json = v["kinds"][kind.name()]["content_slots"]
            .as_array()
            .expect("array");
        let code = kind.content_slots();
        assert_eq!(json.len(), code.len(), "slot count for {kind}");
        for (want, got) in json.iter().zip(code) {
            assert_eq!(
                want["name"].as_str().expect("name"),
                got.slot.name(),
                "slot order for {kind}"
            );
            let required = want["required"].as_bool().unwrap_or(false);
            assert_eq!(
                required, got.required,
                "requiredness of {}.{}",
                kind, got.slot
            );
            let json_values = want.get("values").map(strings);
            let code_values = kind.content_enum_values(got.slot).map(|values| {
                values
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<BTreeSet<_>>()
            });
            assert_eq!(
                json_values, code_values,
                "enum values of {}.{}",
                kind, got.slot
            );
        }
    }
}

#[test]
fn every_content_slot_name_round_trips() {
    for slot in ContentSlot::ALL {
        assert_eq!(ContentSlot::from_name(slot.name()), Some(*slot));
    }
    for kind in Kind::ALL {
        assert_eq!(Kind::from_name(kind.name()), Some(*kind));
    }
    for state in State::ALL {
        assert_eq!(State::from_name(state.name()), Some(*state));
    }
    for relation in Relation::ALL {
        assert_eq!(Relation::from_name(relation.name()), Some(*relation));
    }
}

#[test]
fn every_lifecycle_matches_the_json() {
    let v = vocabulary();
    for class in Class::ALL {
        let lifecycle_name = v["classes"][class.name()]["lifecycle"]
            .as_str()
            .expect("lifecycle name");
        let lifecycle = &v["lifecycles"][lifecycle_name];

        let set = |field: &str| strings(&lifecycle[field]);
        let code = |states: &[State]| -> BTreeSet<String> {
            states.iter().map(|s| s.name().to_owned()).collect()
        };

        assert_eq!(set("states"), code(class.states()), "states of {class}");
        assert_eq!(
            set("initial"),
            code(class.initial()),
            "initial states of {class}"
        );
        assert_eq!(set("live"), code(class.live()), "live states of {class}");
        assert_eq!(
            set("terminal"),
            code(class.terminal()),
            "terminal states of {class}"
        );

        let json_transitions: BTreeSet<(String, String, String)> = lifecycle["transitions"]
            .as_array()
            .expect("array")
            .iter()
            .map(|t| {
                (
                    t["from"].as_str().expect("from").to_owned(),
                    t["to"].as_str().expect("to").to_owned(),
                    t["trigger"].as_str().expect("trigger").to_owned(),
                )
            })
            .collect();
        let code_transitions: BTreeSet<(String, String, String)> = class
            .transitions()
            .iter()
            .map(|t| {
                (
                    t.from.name().to_owned(),
                    t.to.name().to_owned(),
                    t.trigger.to_owned(),
                )
            })
            .collect();
        assert_eq!(json_transitions, code_transitions, "transitions of {class}");
    }
}

#[test]
fn every_relation_is_in_code_and_in_the_json() {
    let v = vocabulary();
    let json = names(&v["relations"]);
    let code: BTreeSet<String> = Relation::ALL.iter().map(|r| r.name().to_owned()).collect();
    assert_eq!(json, code, "relation sets differ");
    assert_eq!(code.len(), 12, "twelve relations");
}

#[test]
fn every_relation_domain_range_and_properties_match() {
    let v = vocabulary();
    for relation in Relation::ALL {
        let entry = &v["relations"][relation.name()];

        match entry["domain"].as_str() {
            Some("any") => assert_eq!(relation.domain(), Domain::Any, "{relation} domain"),
            Some(other) => panic!("unexpected domain {other:?} for {relation}"),
            None => {
                // `verified_by` also names `check`, which is a block rather than a kind
                // and is carried by Check::verified_by instead of the domain table.
                let json: BTreeSet<String> = strings(&entry["domain"])
                    .into_iter()
                    .filter(|k| k != "check")
                    .collect();
                let code: BTreeSet<String> = Kind::ALL
                    .iter()
                    .filter(|k| relation.domain().accepts(**k))
                    .map(|k| k.name().to_owned())
                    .collect();
                assert_eq!(json, code, "{relation} domain");
            }
        }

        match entry["range"].as_str() {
            Some("any") => assert_eq!(relation.range(), Range::Any, "{relation} range"),
            Some(s) if s.starts_with("same kind") => {
                assert_eq!(relation.range(), Range::SameKind, "{relation} range");
            }
            Some(other) => panic!("unexpected range {other:?} for {relation}"),
            None => {
                let json = strings(&entry["range"]);
                let code: BTreeSet<String> = Kind::ALL
                    .iter()
                    .filter(|k| relation.range().accepts(Kind::Work, **k))
                    .map(|k| k.name().to_owned())
                    .collect();
                assert_eq!(json, code, "{relation} range");
            }
        }

        let cardinality = match entry["cardinality"].as_str().expect("cardinality") {
            "one" => Cardinality::One,
            "many" => Cardinality::Many,
            other => panic!("unexpected cardinality {other:?}"),
        };
        assert_eq!(
            relation.cardinality(),
            cardinality,
            "{relation} cardinality"
        );
        assert_eq!(
            relation.acyclic(),
            entry["acyclic"].as_bool().expect("acyclic"),
            "{relation} acyclic"
        );
        assert_eq!(
            relation.propagates_staleness(),
            entry["propagates_staleness"]
                .as_bool()
                .expect("propagates_staleness"),
            "{relation} propagates_staleness"
        );
        assert_eq!(
            relation.symmetric(),
            entry["symmetric"].as_bool().unwrap_or(false),
            "{relation} symmetric"
        );
        assert_eq!(
            relation.enforced_by().to_string(),
            entry["enforced_by"].as_str().expect("enforced_by"),
            "{relation} enforced_by"
        );
    }
}

#[test]
fn every_rule_has_a_function_and_the_json_code() {
    let v = vocabulary();
    let json = names(&v["rules"]);
    let code: BTreeSet<String> = RULES.iter().map(|r| r.id.to_string()).collect();
    assert_eq!(json, code, "rule sets differ");
    assert_eq!(code.len(), 25, "V-001..V-025");

    for spec in RULES {
        let entry = &v["rules"][spec.id.to_string()];
        assert_eq!(
            spec.code.as_str(),
            entry["code"].as_str().expect("code"),
            "primary code for {}",
            spec.id
        );
        let stage = entry["stage"].as_str().expect("stage");
        let expected = match stage {
            "type" => "Type",
            "link" => "Link",
            "resolve" => "Resolve",
            other => panic!("unexpected stage {other:?}"),
        };
        assert_eq!(
            format!("{:?}", spec.stage),
            expected,
            "stage of {}",
            spec.id
        );
    }
}

#[test]
fn scope_term_forms_match() {
    let v = vocabulary();
    assert_eq!(
        v["scope_terms"].as_array().expect("array").len(),
        3,
        "D-010 fixes three scope term forms"
    );
}
