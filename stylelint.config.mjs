export default {
  extends: ["stylelint-config-standard", "stylelint-config-recommended-vue"],
  rules: {
    "selector-class-pattern": "^(?:n-[a-z0-9_-]+|[a-z][a-z0-9]*(?:-[a-z0-9]+)*)$",
    "selector-max-compound-selectors": 5,
    "selector-max-id": 1,
    "selector-max-specificity": "1,5,2",
  },
};
