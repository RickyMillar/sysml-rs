#!/usr/bin/env node
/**
 * Test script to validate highlights.scm queries against the grammar.
 * Run with: node test_highlights.js
 */

const fs = require('fs');
const path = require('path');

// Load the parser
let Parser;
try {
  Parser = require('tree-sitter');
} catch (e) {
  console.error('tree-sitter not installed. Run: npm install tree-sitter');
  process.exit(1);
}

const parser = new Parser();

// Load the language
let SysML;
try {
  SysML = require('./bindings/node');
  parser.setLanguage(SysML);
} catch (e) {
  console.error('SysML grammar not built. Run: npm install && npm run build');
  console.error(e.message);
  process.exit(1);
}

// Test files
const testCases = [
  {
    name: 'Simple package',
    code: `package Test {
  part def Vehicle {
    part engine : Engine;
  }
  part def Engine;
}`,
  },
  {
    name: 'State machine',
    code: `package StateMachine {
  state def States {
    entry { /* init */ }
    state idle;
    state running;
    transition first idle then running;
  }
}`,
  },
  {
    name: 'Action with parameters',
    code: `package Actions {
  action def ProcessData {
    in data : DataType;
    out result : ResultType;
  }
}`,
  },
  {
    name: 'Requirements',
    code: `package Requirements {
  requirement def SpeedLimit {
    subject vehicle : Vehicle;
    require constraint { vehicle.speed <= 100 }
  }
}`,
  },
];

// Highlights query files to test
const highlightsFiles = [
  path.join(__dirname, 'queries/highlights.scm'),
  path.join(__dirname, '../../sysml-lsp-zed-extension/languages/sysml/highlights.scm'),
];

console.log('=== SysML Tree-sitter Grammar Tests ===\n');

// Test parsing
console.log('1. Testing parsing...\n');
let parseErrors = 0;
for (const tc of testCases) {
  const tree = parser.parse(tc.code);
  const hasError = tree.rootNode.hasError();
  const status = hasError ? '❌ FAIL' : '✓ PASS';
  console.log(`  ${status} ${tc.name}`);
  if (hasError) {
    parseErrors++;
    // Find error nodes
    const findErrors = (node, depth = 0) => {
      if (node.type === 'ERROR' || node.isMissing()) {
        console.log(`    Error at line ${node.startPosition.row + 1}: ${node.type}`);
      }
      for (let i = 0; i < node.childCount; i++) {
        findErrors(node.child(i), depth + 1);
      }
    };
    findErrors(tree.rootNode);
  }
}
console.log(`\n  Parsing: ${testCases.length - parseErrors}/${testCases.length} passed\n`);

// Test highlights queries
console.log('2. Testing highlights queries...\n');

for (const hlFile of highlightsFiles) {
  const relativePath = path.relative(process.cwd(), hlFile);
  if (!fs.existsSync(hlFile)) {
    console.log(`  ⚠ SKIP ${relativePath} (file not found)`);
    continue;
  }

  const queryText = fs.readFileSync(hlFile, 'utf-8');

  try {
    // Try to create query - this validates the syntax
    const query = new Parser.Query(SysML, queryText);
    console.log(`  ✓ PASS ${relativePath}`);
    console.log(`         ${query.captureNames.length} captures defined`);

    // Test query on sample code
    const tree = parser.parse(testCases[0].code);
    const captures = query.captures(tree.rootNode);
    console.log(`         ${captures.length} matches on test code`);
  } catch (e) {
    console.log(`  ❌ FAIL ${relativePath}`);
    console.log(`         ${e.message}`);
  }
}

console.log('\n=== Done ===');
process.exit(parseErrors > 0 ? 1 : 0);
