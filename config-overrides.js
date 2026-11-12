/**
 * react-app-rewired overrides for react-scripts 5 (webpack 5).
 *
 * webpack 4 (react-scripts 4) automatically bundled browser polyfills for Node.js core
 * modules. webpack 5 dropped that, but several renderer dependencies still rely on it
 * (universal-analytics -> request, msgpack5, vfile, ...). This restores the same
 * behavior react-scripts 4 had:
 *   - polyfill Node core modules (node-polyfill-webpack-plugin uses the same
 *     browserify-family packages webpack 4 used)
 *   - resolve fs/net/tls/child_process to empty modules, as CRA 4 configured them
 */
const NodePolyfillPlugin = require('node-polyfill-webpack-plugin');

module.exports = function override(config) {
  config.plugins.push(new NodePolyfillPlugin({ additionalAliases: ['punycode'] }));

  config.resolve.fallback = {
    ...config.resolve.fallback,
    fs: false,
    net: false,
    tls: false,
    child_process: false,
  };

  return config;
};
