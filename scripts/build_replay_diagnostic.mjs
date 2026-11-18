import {build} from 'vite';
await build({configFile:false,build:{outDir:'build/replay-diagnostic',emptyOutDir:true,target:'es2021',minify:false,lib:{entry:'src/replay-diagnostic.js',name:'ReplayDiagnostic',formats:['iife'],fileName:()=> 'diagnostic.js'},rollupOptions:{output:{inlineDynamicImports:true}}}});
