# Bundled script libraries

Scripts can `require` these, as in Postman's sandbox. They are loaded only when a script uses them.

| Module      | Version | License    | File                     |
| ----------- | ------- | ---------- | ------------------------ |
| `chai`      | 4.5.0   | MIT        | `chai.js` (minified)     |
| `lodash`    | 4.17.21 | MIT        | `lodash.js`              |
| `moment`    | 2.30.1  | MIT        | `moment.js`              |
| `crypto-js` | 4.2.0   | MIT        | `crypto-js.js` (minified)|
| `ajv`       | 6.15.0  | MIT        | `ajv.js`                 |
| `tv4`       | 1.3.0   | Public domain / MIT | `tv4.js` (minified) |

License texts are in `licenses/`. Files come from the npm packages' browser builds (`chai.js`, `lodash.min.js`,
`min/moment.min.js`, `crypto-js.js`, `dist/ajv.min.js`, `tv4.js`); three were minified with esbuild.
