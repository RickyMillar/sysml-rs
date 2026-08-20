/**
 * Tree-sitter grammar for SysML v2
 *
 * This grammar covers the textual notation for SysML v2 as defined in the
 * SysML v2 specification. It includes expressions, definitions, usages,
 * state machines, and constraints.
 *
 * Structure:
 *   grammar.js          — this file: config, extras, word, conflicts, rule assembly
 *   helpers/patterns.js  — factory functions (defRule, binaryExpr, etc.)
 *   helpers/conflicts.js — programmatic conflict generation
 *   rules/*.js           — grouped rule modules
 *   generated/*.js       — auto-generated keyword/operator/enum constants
 */

module.exports = grammar({
  name: "sysml",

  extras: ($) => [/\s/, $.comment, $.ml_note, $.sl_note],

  word: ($) => $.identifier,

  conflicts: require("./helpers/conflicts"),

  rules: {
    // Assemble all rules from modules
    ...require("./rules/namespaces"),
    ...require("./rules/common"),
    ...require("./rules/definitions"),
    ...require("./rules/usages"),
    ...require("./rules/actions"),
    ...require("./rules/states"),
    ...require("./rules/connectors"),
    ...require("./rules/requirements"),
    ...require("./rules/expressions"),
    ...require("./rules/types"),
    ...require("./rules/kerml"),
  },
});
