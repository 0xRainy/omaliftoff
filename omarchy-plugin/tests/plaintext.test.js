// Every QML Text must render as plain text: launch names, pads, weather notes, errors and news
// come from LL2/SpaceX. With Qt's default (AutoText) a string like "<img src=http://…>" is
// rendered as HTML and the image is fetched (checked with the qml runtime, see CHANGELOG 0.1.5).
const assert = require("assert");
const fs = require("fs"), path = require("path");
let n = 0;
for (const f of ["Panel.qml", "BarWidget.qml"]) {
  const lines = fs.readFileSync(path.join(__dirname, "..", f), "utf8").split("\n");
  lines.forEach((l, i) => {
    const m = /^(\s*)Text \{\s*$/.exec(l);
    if (!m) return;
    let ok = false;
    for (let j = i + 1; j < lines.length && !lines[j].startsWith(m[1] + "}"); j++)
      if (lines[j].startsWith(m[1] + "  textFormat: Text.PlainText")) ok = true;
    assert.ok(ok, `${f}:${i + 1}: Text without textFormat: Text.PlainText`);
    n++;
  });
  assert.ok(!/RichText|StyledText|AutoText/.test(lines.join("\n")), `${f}: rich/auto text format used`);
}
assert.ok(n >= 20, "found only " + n + " Text items");
console.log(`plaintext.test.js: all ${n} Text items are PlainText`);
