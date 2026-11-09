import { defineConfig } from 'vite';
import { fileURLToPath, URL } from 'node:url';
export default defineConfig({ root:fileURLToPath(new URL('.',import.meta.url)), base:'./', build:{outDir:'dist',emptyOutDir:true,target:['es2022','safari15']} });
