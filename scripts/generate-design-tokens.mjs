import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const tokens = JSON.parse(readFileSync(new URL('design/tokens.json', root), 'utf8'));
const colors = Object.entries(tokens.colors);
const metrics = Object.entries(tokens.metrics);
for (const [name, value] of colors) {
  if (!/^[a-zA-Z]+$/u.test(name) || !/^#[0-9a-f]{6}$/u.test(value)) throw new Error(`Invalid color: ${name}`);
}
for (const [name, value] of metrics) {
  if (!/^[a-zA-Z]+$/u.test(name) || !Number.isSafeInteger(value) || value < 1) throw new Error(`Invalid metric: ${name}`);
}
const kebab = (name) => name.replace(/[A-Z]/gu, (letter) => `-${letter.toLowerCase()}`);
const banner = 'Generated from design/tokens.json; run node scripts/generate-design-tokens.mjs.';
const softColors = [['accent-soft', 'accent', '14'], ['accent-border', 'accent', '3d'], ['success-soft', 'success', '1a'], ['warning-soft', 'warning', '1a'], ['danger-soft', 'danger', '1a']];
const softCss = softColors.map(([name, color, alpha]) => `  --sn-${name}: ${tokens.colors[color]}${alpha};`).join('\n');
const outputs = new Map([
  ['apps/desktop/src/styles/tokens.css', `/* ${banner} */\n:root {\n${colors.map(([name, value]) => `  --sn-${kebab(name)}: ${value};`).join('\n')}\n${softCss}\n${metrics.map(([name, value]) => `  --sn-${kebab(name)}: ${value}px;`).join('\n')}\n}\n`],
  ['apps/mobile/lib/src/design_tokens.dart', `// ${banner}\nimport 'package:flutter/painting.dart';\n\nabstract final class SanserTokens {\n${colors.map(([name, value]) => `  static const ${name} = Color(0xff${value.slice(1)});`).join('\n')}\n${metrics.map(([name, value]) => `  static const double ${name} = ${value};`).join('\n')}\n}\n`],
  ['native/common/design_tokens.h', `// ${banner}\n#pragma once\n#include <cstdint>\n\nnamespace sanser::design {\n${colors.map(([name, value]) => `inline constexpr std::uint32_t ${name} = 0x${value.slice(1)};`).join('\n')}\n${metrics.map(([name, value]) => `inline constexpr int ${name} = ${value};`).join('\n')}\n}\n`]
]);
for (const [path, content] of outputs) {
  const url = new URL(path, root);
  if (process.argv.includes('--check')) {
    if (readFileSync(url, 'utf8') !== content) throw new Error(`Stale generated design tokens: ${path}`);
  } else writeFileSync(url, content);
  console.log(`${process.argv.includes('--check') ? 'Verified' : 'Generated'} ${fileURLToPath(url).slice(fileURLToPath(root).length)}`);
}
