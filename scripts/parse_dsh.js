const fs = require('fs');
const html = fs.readFileSync('scratch_deepseek_harness.html', 'utf8');

// Find all css hrefs
const cssMatches = html.match(/href="\/_next\/static\/css\/[^"]+"/g);
console.log('CSS:', cssMatches);

// Let's find sections in HTML
const sectionMatches = html.match(/<section[\s\S]*?<\/section>/g);
console.log('Sections found in raw html:', sectionMatches ? sectionMatches.length : 0);

// Search for Harness translations
const idx = html.indexOf('"Harness":{');
if (idx !== -1) {
  // extract until matching brace or reasonable size
  const slice = html.slice(idx, idx + 15000);
  fs.writeFileSync('scratch_dsh_content.json', '{' + slice.slice(0, slice.lastIndexOf('}') + 1) + '}');
  console.log('Extracted slice around Harness');
}
