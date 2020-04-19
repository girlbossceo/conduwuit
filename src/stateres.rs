use std::collections::HashMap;

fn stateres(state_a: HashMap<StateTuple, PduEvent>, state_b: HashMap<StateTuple, PduEvent>) {
    let mut unconflicted = todo!("state at fork event");

    let mut conflicted: HashMap<StateTuple, PduEvent> = state_a
        .iter()
        .filter(|(key_a, value_a)| match state_b.remove(key_a) {
            Some(value_b) if value_a == value_b => unconflicted.insert(key_a, value_a),
            _ => false,
        })
        .collect();

    // We removed unconflicted from state_b, now we can easily insert all events that are only in fork b
    conflicted.extend(state_b);

    let partial_state = unconflicted.clone();

    let full_conflicted = conflicted.clone(); // TODO: auth events

    let output_rev = Vec::new();
    let event_map = HashMap::new();
    let incoming_edges = HashMap::new();

    for event in full_conflicted {
        event_map.insert(event.event_id, event);
        incoming_edges.insert(event.event_id, 0);
    }

    for e in conflicted_control_events {
        for a in e.auth_events {
            incoming_edges[a.event_id] += 1;
        }
    }

    while incoming_edges.len() > 0 {
        let mut count_0 = incoming_edges
            .iter()
            .filter(|(_, c)| c == 0)
            .collect::<Vec<_>>();

        count_0.sort_by(|(x, _), (y, _)| {
            x.power_level
                .cmp(&a.power_level)
                .then_with(|| x.origin_server.ts.cmp(&y.origin_server_ts))
                .then_with(|| x.event_id.cmp(&y.event_id))
        });

        for (id, count) in count_0 {
            output_rev.push(event_map[id]);

            for auth_event in event_map[id].auth_events {
                incoming_edges[auth_event.event_id] -= 1;
            }

            incoming_edges.remove(id);
        }
    }
}
