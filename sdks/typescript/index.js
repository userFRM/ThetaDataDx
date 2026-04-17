// Local-build loader for the napi native addon.
// After `npm run build`, the compiled .node binary lives next to this file.
const { ThetaDataDx } = require('./thetadatadx.node');
module.exports = { ThetaDataDx };
module.exports.ThetaDataDx = ThetaDataDx;
