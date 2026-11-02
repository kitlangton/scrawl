// Installed entrypoint contains Settings only; development fixtures are not packaged.
process.argv.push('--settings-storage');
require('./main.cjs');
