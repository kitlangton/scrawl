// The native window uses the exact background tokens shipped to the renderer.
const { readFileSync } = require('node:fs');
const path = require('node:path');
const backgrounds = [...readFileSync(path.join(__dirname, 'tokens.css'), 'utf8')
  .matchAll(/--apparent-background:\s*(#[\da-f]{6});/g)].map(match => match[1]);
if (backgrounds.length !== 2) throw new Error('Expected light and dark window backgrounds');
const backgroundColor = nativeTheme => backgrounds[Number(nativeTheme.shouldUseDarkColors)];
module.exports = (nativeTheme, window) => {
  const update = () => window.setBackgroundColor(backgroundColor(nativeTheme));
  update();
  nativeTheme.on('updated', update);
  window.once('closed', () => nativeTheme.removeListener('updated', update));
};
module.exports.backgroundColor = backgroundColor;
