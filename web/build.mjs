import * as esbuild from 'esbuild';

const isWatch = process.argv.includes('--watch');

const buildOptions = {
  entryPoints: ['src/decrypt.ts'],
  bundle: true,
  minify: !isWatch,
  sourcemap: isWatch,
  target: ['es2020'],
  outfile: 'dist/decrypt.js',
  format: 'iife',
};

if (isWatch) {
  const ctx = await esbuild.context(buildOptions);
  await ctx.watch();
  console.log('Watching for changes...');
} else {
  await esbuild.build(buildOptions);
  console.log('Build complete!');
}
