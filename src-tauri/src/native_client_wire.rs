//! Minimal endpoint-generation-1 envelopes (Herdr 0.9.x wire.rs).
//! Render and input frames are forwarded unchanged, never decoded here.
use serde_json::{Value, json};

pub const MAX_FRAME: usize = 64 * 1024 * 1024;
const INVALID: &str = "terminal.client_protocol";

// Bincode 2 standard uses unsigned varints for lengths and enum discriminants.
fn number(bytes: &mut Vec<u8>, value: usize) {
    if value < 251 {
        bytes.push(value as u8);
    } else if value <= u16::MAX as usize {
        bytes.push(251);
        bytes.extend((value as u16).to_le_bytes());
    } else {
        bytes.push(252);
        bytes.extend((value as u32).to_le_bytes());
    }
}
fn take_number(bytes: &mut &[u8]) -> Result<usize, String> {
    let (&tag, rest) = bytes.split_first().ok_or(INVALID)?;
    *bytes = rest;
    let size = match tag {
        0..=250 => return Ok(tag as usize),
        251 => 2,
        252 => 4,
        _ => return Err(INVALID.into()),
    };
    let raw = bytes.get(..size).ok_or(INVALID)?;
    let mut result = [0u8; 4];
    result[..size].copy_from_slice(raw);
    *bytes = &bytes[size..];
    Ok(u32::from_le_bytes(result) as usize)
}
fn string(bytes: &mut Vec<u8>, value: &str) {
    number(bytes, value.len());
    bytes.extend(value.as_bytes());
}
fn take_bytes<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8], String> {
    let size = take_number(bytes)?;
    let value = bytes.get(..size).ok_or(INVALID)?;
    *bytes = &bytes[size..];
    Ok(value)
}
fn take_string<'a>(bytes: &mut &'a [u8]) -> Result<&'a str, String> {
    std::str::from_utf8(take_bytes(bytes)?).map_err(|_| INVALID.into())
}
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut result = (payload.len() as u32).to_le_bytes().to_vec();
    result.extend(payload);
    result
}
pub fn next_frame(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let size = u32::from_le_bytes(buffer[..4].try_into().unwrap()) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(INVALID.into());
    }
    if buffer.len() < size + 4 {
        return Ok(None);
    }
    let payload = buffer[4..size + 4].to_vec();
    buffer.drain(..size + 4);
    Ok(Some(payload))
}
pub fn control(payload: &[u8]) -> Result<Option<(&str, Value)>, String> {
    if payload.first() != Some(&20) {
        return Ok(None);
    }
    let mut bytes = &payload[1..];
    let kind = take_string(&mut bytes)?;
    let data = take_string(&mut bytes)?;
    if !bytes.is_empty() {
        return Err(INVALID.into());
    }
    // Other named controls may use opaque non-JSON data.
    if !matches!(
        kind,
        "endpoint.hello.v1" | "endpoint.welcome.v1" | "shell.snapshot.v1"
    ) {
        return Ok(None);
    }
    Ok(Some((
        kind,
        serde_json::from_str(data).map_err(|_| INVALID)?,
    )))
}
pub fn zoom(boot: &str, id: &str, pane: &str) -> Vec<u8> {
    // Explicit On is idempotent and also focuses the requested pane in Herdr.
    let request =
        json!({"id": id, "method": "pane.zoom", "params": {"pane_id": pane, "mode": "on"}})
            .to_string();
    let mut payload = vec![15];
    string(&mut payload, boot);
    string(&mut payload, &request);
    frame(&payload)
}
pub fn response<'a>(
    payload: &'a [u8],
    boot: &str,
    id: &str,
) -> Result<Option<(bool, &'a [u8])>, String> {
    if payload.first() != Some(&18) {
        return Ok(None);
    }
    let mut bytes = &payload[1..];
    let actual_boot = take_string(&mut bytes)?;
    let actual_id = take_string(&mut bytes)?;
    if actual_id != id {
        return Ok(None);
    }
    if actual_boot != boot {
        return Err(INVALID.into());
    }
    let (&last, rest) = bytes.split_first().ok_or(INVALID)?;
    if last > 1 {
        return Err(INVALID.into());
    }
    bytes = rest;
    let data = take_bytes(&mut bytes)?;
    if !bytes.is_empty() {
        return Err(INVALID.into());
    }
    Ok(Some((last == 1, data)))
}

pub struct Position {
    id: String,
    pane: String,
    boot: Option<String>,
    welcome: bool,
    reply: Vec<u8>,
    acknowledged: bool,
    target_visible: bool,
    pub ready: bool,
}
impl Position {
    pub fn new(pane: String) -> Self {
        Self {
            id: String::new(),
            pane,
            boot: None,
            welcome: false,
            reply: Vec::new(),
            acknowledged: false,
            target_visible: false,
            ready: false,
        }
    }
    fn request(&mut self) -> Vec<u8> {
        self.id = format!("kanban-zoom-{}", uuid::Uuid::new_v4());
        self.reply.clear();
        self.acknowledged = false;
        self.ready = false;
        zoom(self.boot.as_ref().unwrap(), &self.id, &self.pane)
    }

    // All panes (including hidden panes) are present in shell snapshots.
    fn visible_alone(&self, doc: &Value) -> Result<bool, String> {
        let panes = doc["panes"].as_array().ok_or(INVALID)?;
        let target = panes
            .iter()
            .find(|p| p["pane_id"] == self.pane)
            .ok_or("terminal.client_focus")?;
        let tab_id = target["tab_id"].as_str().ok_or(INVALID)?;
        let tab = doc["tabs"]
            .as_array()
            .ok_or(INVALID)?
            .iter()
            .find(|t| t["tab_id"] == tab_id)
            .ok_or(INVALID)?;
        let zoomed = tab["zoomed"].as_bool().ok_or(INVALID)?;
        let single = panes.iter().filter(|p| p["tab_id"] == tab_id).count() == 1;
        Ok(doc["focused_pane_id"] == self.pane && (zoomed || single))
    }

    // Return a serialized upstream request and whether to forward the server frame.
    pub fn receive(&mut self, payload: &[u8]) -> Result<(Option<Vec<u8>>, bool), String> {
        if let Some((kind, doc)) = control(payload)? {
            if kind == "endpoint.welcome.v1" {
                if doc["generation"] != 1
                    || !doc["error"].is_null()
                    || !doc["methods"]
                        .as_array()
                        .is_some_and(|m| m.iter().any(|s| s == "pane.zoom"))
                {
                    return Err(INVALID.into());
                }
                self.welcome = true;
            }
            if kind == "shell.snapshot.v1" {
                let boot = doc["boot_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(INVALID)?;
                if !self.welcome {
                    return Err(INVALID.into());
                }
                self.target_visible = self.visible_alone(&doc)?;
                if let Some(expected) = &self.boot {
                    if expected != boot {
                        return Err(INVALID.into());
                    }
                    if self.ready {
                        // A no-focus split clears zoom. Restore it while this task
                        // remains selected, without undoing manual Herdr navigation.
                        if doc["focused_pane_id"] == self.pane && !self.target_visible {
                            return Ok((Some(self.request()), true));
                        }
                    } else {
                        self.ready = self.acknowledged && self.target_visible;
                    }
                } else {
                    self.boot = Some(boot.into());
                    return Ok((Some(self.request()), true));
                }
            }
        }
        if let Some(boot) = &self.boot {
            if let Some((last, data)) = response(payload, boot, &self.id)? {
                if self.reply.len() + data.len() > 1024 * 1024 {
                    return Err(INVALID.into());
                }
                self.reply.extend(data);
                if last {
                    let doc: Value = serde_json::from_slice(&self.reply).map_err(|_| INVALID)?;
                    let result = &doc["result"]["zoom"];
                    let layout = &result["layout"];
                    let single = result["reason"] == "single_pane"
                        && layout["panes"].as_array().is_some_and(|panes| {
                            panes.len() == 1 && panes[0]["pane_id"] == self.pane
                        });
                    if doc["id"] != self.id
                        || !doc["error"].is_null()
                        || result["pane_id"] != self.pane
                        || result["focused_pane_id"] != self.pane
                        || layout["focused_pane_id"] != self.pane
                        || !(single || (result["zoomed"] == true && layout["zoomed"] == true))
                    {
                        return Err("terminal.client_focus".into());
                    }
                    self.acknowledged = true;
                    self.ready = self.target_visible;
                }
                return Ok((None, false)); // The official client did not issue our request.
            }
        }
        Ok((None, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn control_frame(kind: &str, doc: Value) -> Vec<u8> {
        let mut v = vec![20];
        string(&mut v, kind);
        string(&mut v, &doc.to_string());
        v
    }
    fn snapshot(pane: &str, zoomed: bool, count: usize) -> Vec<u8> {
        control_frame(
            "shell.snapshot.v1",
            json!({
                "boot_id":"boot", "focused_pane_id":pane,
                "tabs":[{"tab_id":"w2:t1","zoomed":zoomed}],
                "panes": (0..count).map(|i| json!({"pane_id":format!("w2:p{}", i+3),"tab_id":"w2:t1"})).collect::<Vec<_>>()
            }),
        )
    }
    fn reply_doc(position: &Position, single: bool) -> Value {
        json!({"id":position.id,"result":{"zoom":{
            "pane_id":position.pane,"focused_pane_id":position.pane,
            "zoomed":!single,"reason":if single { "single_pane" } else { "already_zoomed" },
            "layout":{"focused_pane_id":position.pane,"zoomed":!single,
                "panes":[{"pane_id":position.pane}]}
        }}})
    }
    fn reply(position: &Position, doc: Value) -> Vec<u8> {
        let mut payload = vec![18];
        string(&mut payload, "boot");
        string(&mut payload, &position.id);
        payload.push(1);
        string(&mut payload, &doc.to_string());
        payload
    }
    fn position() -> Position {
        let mut position = Position::new("w2:p3".into());
        position
            .receive(&control_frame(
                "endpoint.welcome.v1",
                json!({"generation":1,"methods":["pane.zoom"]}),
            ))
            .unwrap();
        position
    }
    #[test]
    fn requires_own_successful_reply_and_target_snapshot() {
        let mut position = position();
        let request = position
            .receive(&snapshot("w1:p1", false, 2))
            .unwrap()
            .0
            .unwrap();
        let mut buffer = request;
        let payload = next_frame(&mut buffer).unwrap().unwrap();
        assert_eq!(payload[0], 15);
        let mut bytes = &payload[1..];
        assert_eq!(take_string(&mut bytes).unwrap(), "boot");
        let request: Value = serde_json::from_str(take_string(&mut bytes).unwrap()).unwrap();
        assert_eq!(request["method"], "pane.zoom");
        assert_eq!(request["params"], json!({"pane_id":"w2:p3", "mode":"on"}));
        position.receive(&snapshot("w2:p3", true, 2)).unwrap();
        assert!(!position.ready);
        let reply = reply_doc(&position, false).to_string();
        for (last, part) in [(false, &reply[..20]), (true, &reply[20..])] {
            let mut payload = vec![18];
            string(&mut payload, "boot");
            string(&mut payload, &position.id);
            payload.push(last as u8);
            string(&mut payload, part);
            assert!(!position.receive(&payload).unwrap().1);
        }
        assert!(position.ready);
        assert!(
            position
                .receive(&control_frame(
                    "shell.snapshot.v1",
                    json!({"boot_id":"replacement"})
                ))
                .is_err()
        );
    }
    #[test]
    fn framing_handles_fragmentation_and_rejects_oversized_frames() {
        let payload = vec![9; 66000];
        let mut input = frame(&payload);
        let tail = input.split_off(100);
        assert!(next_frame(&mut input).unwrap().is_none());
        input.extend(tail);
        assert_eq!(next_frame(&mut input).unwrap().unwrap(), payload);
        assert!(next_frame(&mut u32::MAX.to_le_bytes().to_vec()).is_err());
        for text in ["a".repeat(250), "界".repeat(100), "a".repeat(66000)] {
            let mut data = Vec::new();
            string(&mut data, &text);
            let mut bytes = data.as_slice();
            assert_eq!(take_string(&mut bytes).unwrap(), text);
            assert!(bytes.is_empty());
        }
    }

    #[test]
    fn initial_target_needs_no_redundant_snapshot_but_requires_success() {
        for success in [true, false] {
            let mut position = position();
            position.receive(&snapshot("w2:p3", true, 2)).unwrap();
            assert!(!position.ready);
            let doc = if success {
                reply_doc(&position, false)
            } else {
                json!({"id":position.id,"error":{"code":"pane_not_found"}})
            };
            let payload = reply(&position, doc);
            assert_eq!(position.receive(&payload).is_ok(), success);
            assert_eq!(position.ready, success);
        }
        let mut position = Position::new("w1:p1".into());
        assert!(
            position
                .receive(&control_frame(
                    "endpoint.welcome.v1",
                    json!({"generation":2,"methods":["pane.zoom"]})
                ))
                .is_err()
        );
        assert!(
            position
                .receive(&control_frame(
                    "endpoint.welcome.v1",
                    json!({"generation":1,"methods":["pane.focus"]})
                ))
                .is_err()
        );
    }

    #[test]
    fn focus_without_zoom_is_not_ready_and_reply_cannot_replace_snapshot() {
        let mut position = position();
        position.receive(&snapshot("w2:p3", false, 2)).unwrap();
        position
            .receive(&reply(&position, reply_doc(&position, false)))
            .unwrap();
        assert!(!position.ready);
        position.receive(&snapshot("w2:p3", false, 2)).unwrap();
        assert!(!position.ready);
        position.receive(&snapshot("w2:p4", true, 2)).unwrap();
        assert!(!position.ready);
        position.receive(&snapshot("w2:p3", true, 2)).unwrap();
        assert!(position.ready);
    }

    #[test]
    fn single_pane_then_background_split_rezooms_once_with_a_new_request() {
        let mut position = position();
        position.receive(&snapshot("w2:p3", false, 1)).unwrap();
        let old_reply = reply(&position, reply_doc(&position, true));
        position.receive(&old_reply).unwrap();
        assert!(position.ready);
        assert!(
            position
                .receive(&snapshot("w2:p3", false, 1))
                .unwrap()
                .0
                .is_none()
        );
        assert!(
            position
                .receive(&snapshot("w2:p3", false, 2))
                .unwrap()
                .0
                .is_some()
        );
        assert!(!position.ready);
        assert!(
            position
                .receive(&snapshot("w2:p3", false, 2))
                .unwrap()
                .0
                .is_none()
        );
        position.receive(&old_reply).unwrap();
        position.receive(&snapshot("w2:p3", true, 2)).unwrap();
        assert!(!position.ready); // An old acknowledgement cannot finish this request.
        position
            .receive(&reply(&position, reply_doc(&position, false)))
            .unwrap();
        assert!(position.ready);
        assert!(
            position
                .receive(&snapshot("w2:p3", true, 3))
                .unwrap()
                .0
                .is_none()
        );
        // Native navigation remains available after initial task positioning.
        assert!(
            position
                .receive(&snapshot("w2:p4", false, 3))
                .unwrap()
                .0
                .is_none()
        );
        assert!(position.ready);
    }

    #[test]
    fn rejects_zoom_failures_wrong_target_and_false_single_pane_success() {
        for case in 0..4 {
            let mut position = position();
            position.receive(&snapshot("w2:p3", false, 2)).unwrap();
            let mut doc = reply_doc(&position, false);
            match case {
                0 => doc["result"]["zoom"]["zoomed"] = json!(false),
                1 => doc["result"]["zoom"]["focused_pane_id"] = json!("w2:p4"),
                2 => doc["result"]["zoom"]["layout"]["zoomed"] = json!(false),
                _ => {
                    doc = reply_doc(&position, true);
                    doc["result"]["zoom"]["layout"]["panes"] = json!([]);
                }
            }
            assert!(position.receive(&reply(&position, doc)).is_err());
            assert!(!position.ready);
        }
    }
}
