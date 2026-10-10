import assert from "node:assert/strict";
import test from "node:test";
import { hasHardLineBreak, isHttpSource, isValidFileName, suggestedFileName } from "../src/downloadDraft.ts";

test("suggests a decoded URL path basename without treating a query as a filename", () => {
  assert.equal(suggestedFileName("https://example.com/files/%E6%8A%A5%E5%91%8A%20v2.zip?token=abc"), "报告 v2.zip");
  assert.equal(suggestedFileName("https://example.com/download?name=report.zip"), "download");
  assert.equal(suggestedFileName("https://example.com/"), "");
  assert.equal(suggestedFileName("https://example.com/folder/"), "");
  assert.equal(suggestedFileName("https://example.com/a%2Fb.zip"), "");
  assert.equal(suggestedFileName("https:example.com/file.zip"), "");
  assert.equal(suggestedFileName("https:///example.com/file.zip"), "");
  assert.equal(suggestedFileName("not a URL"), "");
});

test("rejects paths and control characters in editable filenames", () => {
  for (const name of ["", "  ", ".", "..", "../secret", "dir/file", "dir\\file", "bad\nname", "bad\u0000name"]) {
    assert.equal(isValidFileName(name), false, name);
  }
  assert.equal(isValidFileName("My report 2026.zip"), true);
  assert.equal(isValidFileName("报告.zip"), true);
});

test("URL textarea hard line breaks are rejected", () => {
  assert.equal(hasHardLineBreak("https://example.com/a.zip"), false);
  assert.equal(hasHardLineBreak("https://example.com/a.zip\nhttps://example.com/b.zip"), true);
  assert.equal(hasHardLineBreak("https://example.com/a.zip\r"), true);
  assert.equal(suggestedFileName("https://example.com/a.zip\n"), "");
  assert.equal(isHttpSource("https://example.com/a.zip"), true);
  assert.equal(isHttpSource("ftp://example.com/a.zip"), false);
  assert.equal(isHttpSource("https:example.com/a.zip"), false);
  assert.equal(isHttpSource("https:///example.com/a.zip"), false);
  assert.equal(isHttpSource("https://example.com/a.zip\nhttps://example.com/b.zip"), false);
});
