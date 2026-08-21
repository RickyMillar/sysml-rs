//! JSON-Schema validation gate. Every exported card/example/support/
//! evidence/denominator record is validated against its committed JSON Schema.
//!
//! This is a compact, self-contained validator that interprets the *actual*
//! schema files (the schema stays the single source of truth — this is a
//! generic interpreter, not a hand-coded mirror). It covers exactly the
//! draft-2020-12 keyword subset the five language-pack schemas use: `type`,
//! `enum`, `const`, `pattern`, `required`, `properties`, `additionalProperties`
//! (false), `items`, `minItems`, `minLength`, `maxLength`, `minimum`,
//! `uniqueItems`, `oneOf`, `anyOf`, `allOf`, `not`, `if`/`then`/`else`, and
//! `$ref`/`$defs` (local and sibling-file). Patterns use the already-compiled
//! `regex` crate (no network-fetched schema crate is pulled).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use super::LpError;

/// The five schema files, keyed by the `$id`/filename used in `$ref`s.
pub struct SchemaSet {
    schemas: BTreeMap<String, Value>,
}

const SCHEMA_FILES: &[&str] = &[
    "language-card.schema.json",
    "example.schema.json",
    "support-status.schema.json",
    "evidence-record.schema.json",
    "denominator-record.schema.json",
];

impl SchemaSet {
    /// Load the committed schema files from `tools/spec-index/schemas/`.
    pub fn load(repo_root: &Path) -> Result<Self, LpError> {
        let dir = repo_root.join("tools/spec-index/schemas");
        let mut schemas = BTreeMap::new();
        for name in SCHEMA_FILES {
            let path = dir.join(name);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| LpError::Io(format!("read {}: {e}", path.display())))?;
            let value: Value = serde_json::from_str(&text)
                .map_err(|e| LpError::Schema(format!("parse {name}: {e}")))?;
            schemas.insert((*name).to_owned(), value);
        }
        Ok(SchemaSet { schemas })
    }

    /// Validate `instance` against the named schema file. Returns every
    /// violation found (empty = valid).
    pub fn validate(&self, schema_name: &str, instance: &Value) -> Result<(), Vec<String>> {
        let root = self
            .schemas
            .get(schema_name)
            .ok_or_else(|| vec![format!("unknown schema '{schema_name}'")])?;
        let mut errors = Vec::new();
        self.check(instance, root, schema_name, "$", &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// True iff `instance` validates against `schema` (no error collection).
    fn matches(&self, instance: &Value, schema: &Value, root: &str) -> bool {
        let mut errors = Vec::new();
        self.check(instance, schema, root, "$", &mut errors);
        errors.is_empty()
    }

    fn resolve_ref<'a>(&'a self, reference: &str, current_root: &str) -> Option<(&'a Value, String)> {
        let (file, fragment) = match reference.split_once('#') {
            Some((f, frag)) => (f, frag),
            None => (reference, ""),
        };
        let root_name = if file.is_empty() {
            current_root.to_owned()
        } else {
            file.to_owned()
        };
        let root = self.schemas.get(&root_name)?;
        let mut node = root;
        for raw in fragment.split('/').filter(|s| !s.is_empty()) {
            let token = raw.replace("~1", "/").replace("~0", "~");
            node = node.get(&token)?;
        }
        Some((node, root_name))
    }

    #[allow(clippy::too_many_lines)]
    fn check(&self, inst: &Value, schema: &Value, root: &str, path: &str, errors: &mut Vec<String>) {
        let Some(obj) = schema.as_object() else {
            return; // `true`/`false` schemas are not used here
        };

        // $ref (draft 2020-12: applies alongside sibling keywords).
        if let Some(Value::String(reference)) = obj.get("$ref") {
            if let Some((target, new_root)) = self.resolve_ref(reference, root) {
                self.check(inst, target, &new_root, path, errors);
            } else {
                errors.push(format!("{path}: unresolvable $ref '{reference}'"));
            }
        }

        // type
        if let Some(t) = obj.get("type") {
            let ok = match t {
                Value::String(s) => type_matches(inst, s),
                Value::Array(arr) => arr
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|s| type_matches(inst, s)),
                _ => true,
            };
            if !ok {
                errors.push(format!("{path}: type mismatch (want {t}, got {})", kind_of(inst)));
            }
        }

        // enum
        if let Some(Value::Array(choices)) = obj.get("enum") {
            if !choices.iter().any(|c| c == inst) {
                errors.push(format!("{path}: value {inst} not in enum"));
            }
        }
        // const
        if let Some(c) = obj.get("const") {
            if inst != c {
                errors.push(format!("{path}: value {inst} != const {c}"));
            }
        }

        // string constraints
        if let Some(s) = inst.as_str() {
            if let Some(p) = obj.get("pattern").and_then(Value::as_str) {
                match regex::Regex::new(p) {
                    Ok(re) => {
                        if !re.is_match(s) {
                            errors.push(format!("{path}: '{s}' does not match /{p}/"));
                        }
                    }
                    Err(e) => errors.push(format!("{path}: bad pattern /{p}/: {e}")),
                }
            }
            if let Some(min) = obj.get("minLength").and_then(Value::as_u64) {
                if (s.chars().count() as u64) < min {
                    errors.push(format!("{path}: shorter than minLength {min}"));
                }
            }
            if let Some(max) = obj.get("maxLength").and_then(Value::as_u64) {
                if (s.chars().count() as u64) > max {
                    errors.push(format!("{path}: longer than maxLength {max}"));
                }
            }
        }

        // number constraints
        if let Some(n) = inst.as_f64() {
            if let Some(min) = obj.get("minimum").and_then(Value::as_f64) {
                if n < min {
                    errors.push(format!("{path}: {n} < minimum {min}"));
                }
            }
        }

        // array constraints
        if let Some(arr) = inst.as_array() {
            if let Some(min) = obj.get("minItems").and_then(Value::as_u64) {
                if (arr.len() as u64) < min {
                    errors.push(format!("{path}: fewer than minItems {min}"));
                }
            }
            if obj.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
                for (i, a) in arr.iter().enumerate() {
                    if arr.iter().skip(i + 1).any(|b| b == a) {
                        errors.push(format!("{path}: duplicate array items (uniqueItems)"));
                        break;
                    }
                }
            }
            if let Some(items) = obj.get("items") {
                for (i, el) in arr.iter().enumerate() {
                    self.check(el, items, root, &format!("{path}[{i}]"), errors);
                }
            }
        }

        // object constraints
        if let Some(map) = inst.as_object() {
            if let Some(Value::Array(req)) = obj.get("required") {
                for r in req.iter().filter_map(Value::as_str) {
                    if !map.contains_key(r) {
                        errors.push(format!("{path}: missing required '{r}'"));
                    }
                }
            }
            let props = obj.get("properties").and_then(Value::as_object);
            if let Some(props) = props {
                for (k, v) in map {
                    if let Some(sub) = props.get(k) {
                        self.check(v, sub, root, &format!("{path}.{k}"), errors);
                    }
                }
            }
            if obj.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
                let allowed: Vec<&String> = props.map(|p| p.keys().collect()).unwrap_or_default();
                for k in map.keys() {
                    if !allowed.contains(&k) {
                        errors.push(format!("{path}: additional property '{k}' not allowed"));
                    }
                }
            }
        }

        // combinators
        if let Some(Value::Array(subs)) = obj.get("allOf") {
            for sub in subs {
                self.check(inst, sub, root, path, errors);
            }
        }
        if let Some(Value::Array(subs)) = obj.get("anyOf") {
            if !subs.iter().any(|s| self.matches(inst, s, root)) {
                errors.push(format!("{path}: matches none of anyOf"));
            }
        }
        if let Some(Value::Array(subs)) = obj.get("oneOf") {
            let n = subs.iter().filter(|s| self.matches(inst, s, root)).count();
            if n != 1 {
                errors.push(format!("{path}: matched {n} of oneOf (want exactly 1)"));
            }
        }
        if let Some(sub) = obj.get("not") {
            if self.matches(inst, sub, root) {
                errors.push(format!("{path}: matched a `not` schema"));
            }
        }
        if let Some(cond) = obj.get("if") {
            if self.matches(inst, cond, root) {
                if let Some(then) = obj.get("then") {
                    self.check(inst, then, root, path, errors);
                }
            } else if let Some(els) = obj.get("else") {
                self.check(inst, els, root, path, errors);
            }
        }
    }
}

fn type_matches(inst: &Value, ty: &str) -> bool {
    match ty {
        "object" => inst.is_object(),
        "array" => inst.is_array(),
        "string" => inst.is_string(),
        "boolean" => inst.is_boolean(),
        "null" => inst.is_null(),
        "integer" => inst.is_i64() || inst.is_u64(),
        "number" => inst.is_number(),
        _ => true,
    }
}

fn kind_of(inst: &Value) -> &'static str {
    match inst {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
