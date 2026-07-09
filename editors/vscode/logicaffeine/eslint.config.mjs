import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/**", "out/**", "out-test/**", "node_modules/**", ".vscode-test/**"] },
  ...tseslint.configs.recommended,
  {
    rules: {
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    },
  },
);
