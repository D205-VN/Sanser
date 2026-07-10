import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  { ignores: ['dist/', 'src-tauri/target/', '.svelte-kit/'] },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  ...svelte.configs['flat/recommended'],
  {
    files: ['**/*.ts'],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname
      }
    },
    rules: {
      'no-undef': 'off',
      'svelte/prefer-writable-derived': 'off',
      'svelte/require-each-key': 'off',
      '@typescript-eslint/no-confusing-void-expression': 'off',
      '@typescript-eslint/no-misused-promises': ['error', { checksVoidReturn: false }],
      '@typescript-eslint/no-unnecessary-type-assertion': 'off',
      '@typescript-eslint/no-unsafe-argument': 'off',
      '@typescript-eslint/no-unsafe-assignment': 'off',
      '@typescript-eslint/restrict-template-expressions': ['error', { allowNumber: true }]
    }
  },
  {
    files: ['**/*.svelte'],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
        projectService: true,
        extraFileExtensions: ['.svelte'],
        tsconfigRootDir: import.meta.dirname
      }
    },
    rules: {
      'no-undef': 'off',
      'svelte/prefer-writable-derived': 'off',
      'svelte/require-each-key': 'off',
      '@typescript-eslint/no-confusing-void-expression': 'off',
      '@typescript-eslint/no-misused-promises': ['error', { checksVoidReturn: false }],
      '@typescript-eslint/no-unnecessary-type-assertion': 'off',
      '@typescript-eslint/no-unsafe-argument': 'off',
      '@typescript-eslint/no-unsafe-assignment': 'off',
      '@typescript-eslint/restrict-template-expressions': ['error', { allowNumber: true }]
    }
  },
  {
    files: ['**/*.js'],
    ...tseslint.configs.disableTypeChecked
  }
);
