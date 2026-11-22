// Where Zeno is, fixed when the extension is built (see scripts/build.mjs). The extension asks the browser
// for access to the API's address and to no other site.

declare const __ZENO_API_URL__: string;
declare const __ZENO_WEB_URL__: string;

export const API_URL: string = __ZENO_API_URL__;
export const WEB_URL: string = __ZENO_WEB_URL__;
