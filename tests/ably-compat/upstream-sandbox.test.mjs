import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { createAppDefinition, renderConfig } from './upstream-sandbox.mjs';

const template = readFileSync(new URL('../../config/config.toml', import.meta.url), 'utf8');

test('renders all fixture keys onto one isolated app', () => {
  const definition = createAppDefinition({
    keys: [
      {},
      { capability: { 'chat:*': ['publish'] }, revocableTokens: true },
    ],
    namespaces: [{ id: 'chat', mutableMessages: true }],
  });
  const rendered = renderConfig(template, 7123, definition);

  assert.match(rendered, /^port = 7123$/m);
  assert.match(rendered, new RegExp(`id = "${definition.appId}"`));
  assert.match(rendered, /name = "chat"/);
  assert.match(rendered, /annotations_enabled = true/);
  assert.equal((rendered.match(/^\[ably_compat\]$/gm) ?? []).length, 1);
  assert.equal((rendered.match(/\[\[ably_compat\.keys\]\]/g) ?? []).length, 2);
  assert.match(rendered, /capability = "\{\\"chat:\*\\":\[\\"publish\\"\]\}"/);
  assert.match(rendered, /revocable_tokens = true/);
  assert.match(rendered, /stats_fixture_ingest_enabled = true/);
});

test('uses a full-capability response when a fixture omits capability', () => {
  const definition = createAppDefinition({ keys: [{}] });

  assert.equal(definition.responseKeys[0].capability, '{"*":["*"]}');
  assert.equal(definition.configKeys[0].capability, undefined);
});
