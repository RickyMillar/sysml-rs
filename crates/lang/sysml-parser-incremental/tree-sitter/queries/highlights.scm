[
  "about"
  "abstract"
  "accept"
  "action"
  "actor"
  "after"
  "alias"
  "all"
  "allocate"
  "allocation"
  "analysis"
  "and"
  "as"
  "assert"
  "assign"
  "assoc"
  "assume"
  "attribute"
  "behavior"
  "bind"
  "binding"
  "bool"
  "by"
  "calc"
  "case"
  "chains"
  "class"
  "classifier"
  "collect"
  "comment"
  "composite"
  "concern"
  "connect"
  "connection"
  "connector"
  "constant"
  "constraint"
  "crosses"
  "datatype"
  "decide"
  "def"
  "default"
  "conjugates"
  "dependency"
  "derived"
  "differences"
  "disjoint"
  "do"
  "doc"
  "else"
  "end"
  "entry"
  "enum"
  "event"
  "exhibit"
  "exit"
  "expose"
  "expr"
  "feature"
  "featured"
  "filter"
  "first"
  "flow"
  "for"
  "fork"
  "frame"
  "from"
  "function"
  "hastype"
  "if"
  "implies"
  "import"
  "in"
  "include"
  "individual"
  "inout"
  "interaction"
  "interface"
  "intersects"
  "inv"
  "inverse"
  "istype"
  "item"
  "join"
  "language"
  "library"
  "locale"
  "member"
  "merge"
  "message"
  "meta"
  "metaclass"
  "metadata"
  "namespace"
  "new"
  "nonunique"
  "not"
  "objective"
  "occurrence"
  "of"
  "or"
  "ordered"
  "out"
  "package"
  "parallel"
  "part"
  "perform"
  "port"
  "portion"
  "predicate"
  "private"
  "protected"
  "public"
  "readonly"
  "redefines"
  "ref"
  "references"
  "render"
  "rendering"
  "rep"
  "require"
  "requirement"
  "return"
  "satisfy"
  "select"
  "send"
  "snapshot"
  "specializes"
  "stakeholder"
  "standard"
  "start"
  "state"
  "step"
  "struct"
  "subject"
  "subset"
  "subsets"
  "succession"
  "that"
  "terminate"
  "then"
  "timeslice"
  "to"
  "transition"
  "type"
  "unions"
  "until"
  "use"
  "variant"
  "variation"
  "verification"
  "verify"
  "via"
  "view"
  "viewpoint"
  "when"
  "while"
  "xor"
] @keyword

"self" @variable.builtin
"this" @variable.builtin

(comment) @comment

[
  "{"
  "}"
] @punctuation.bracket

["true" "false"] @constant.builtin
(null_literal) @constant.builtin

(package_decl name: (identifier) @module)
(import_decl target: (import_target) @string.special)
(expose_decl target: (import_target) @string.special)
(render_usage name: (identifier) @variable)

(standard_def name: (identifier) @type)
(case_def name: (identifier) @type)
(definition keyword: _ @keyword)
(definition name: (identifier) @type)

(standard_usage name: (identifier) @variable)
(case_usage name: (identifier) @variable)
(include_use_case_usage name: (identifier) @variable)
(actor_usage name: (identifier) @variable)
(usage keyword: _ @keyword)
(usage name: (identifier) @variable)

; Named rules
(flow_connection_usage name: (identifier) @variable)
(assert_constraint_usage name: (identifier) @variable)
(target_transition_usage target: (identifier) @variable)

; Metadata annotations / user-defined keywords
(metadata_usage "@" @keyword)
(metadata_usage type: (type_ref) @type)
(prefix_metadata_annotation "#" @keyword)
(prefix_metadata_annotation type: (type_ref) @type)

; Feature specialization keywords
(inverting "inverse" @keyword)
(inverting "of" @keyword)
(featuring "featured" @keyword)
(featuring "by" @keyword)

(type_ref (identifier) @type)
(qualified_name (identifier) @type)

(typing ":" @punctuation.delimiter)
(qualified_name "::" @punctuation.delimiter)
