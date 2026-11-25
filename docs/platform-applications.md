# Platform applications: what has to be filed, and what each one needs

Live streaming (M3) and the studio's installer (M2) depend on three things that are decided by other
companies and take time: Google's review of the YouTube permissions, a Twitch developer application, and a
certificate to sign the Windows installer. This page is what was checked on 25 Nov 2026, what each needs from
Zeno, and drafts of what has to be written. **Nothing here has been submitted**: each needs an account,
a domain or a legal entity that only the owner can provide (see *What only the owner can do*).

Facts below are from the cited pages on the date above. Where a page could not be read or said nothing, the
line says so, and it must be checked in the console before filing.

## 1. Google: permission to manage a person's YouTube broadcasts

**What Zeno does with it.** A person connects their YouTube channel once; the studio then creates the
broadcast and its stream, starts and ends it, and (later) reads its live chat into Zeno's chat view. It never
reads or changes anything else on the channel.

**Permissions to ask for (narrowest that works).**

| Scope | Why | Check before filing |
| --- | --- | --- |
| `https://www.googleapis.com/auth/youtube.force-ssl` | Create the broadcast and stream, bind them, and start/end the broadcast | The references for [broadcast creation](https://developers.google.com/youtube/data/live/reference/rest/v3/liveBroadcasts/insert), [stream creation](https://developers.google.com/youtube/data/live/reference/rest/v3/liveStreams/insert), [binding](https://developers.google.com/youtube/data/live/reference/rest/v3/liveBroadcasts/bind) and [transitions](https://developers.google.com/youtube/data/live/reference/rest/v3/liveBroadcasts/transition) list this scope. Confirm its classification in Google Auth Platform → Data Access. |

**Is it reviewed?** Google says sensitive scopes need its review before accounts can grant them, and gives
deleting a YouTube video as an example of a sensitive action
([Sensitive scope verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification)).
The scopes reference does not label each YouTube scope
([OAuth scopes](https://developers.google.com/identity/protocols/oauth2/scopes#youtube)); the label is shown
in the Cloud Console's OAuth consent screen once the scope is added. Record the classification before
filing. If it is restricted, check the [restricted scope requirements](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification)
against the actual data flow. Storing or transmitting restricted data through a server can trigger an
independent security assessment.

**What the review asks for** (same page; the typical time Google gives is 3 to 5 business days, with no
promise):

* a homepage and a **privacy policy on the same domain**, and the app's actual data use must match the policy;
* a published branding status (name, logo, authorised domain) before verification can be requested;
* a justification for each scope, including why a narrower one is not enough;
* an **unlisted YouTube demo video** showing the whole flow: the person starting from Zeno, the OAuth consent
  screen in full, what Zeno then does with the access.

**Quota is a second, separate gate.** Live Streaming methods are part of the YouTube Data API and count against
its default allowance of 10,000 units a day, shared by every user of the project
([quota costs](https://developers.google.com/youtube/v3/determine_quota_cost)). Costs read from that page:
`liveBroadcasts` insert, bind, transition, update, delete and list 1 each; `liveStreams` insert, list, update,
delete 1 each. Going live is about 5 units. Chat `list` and `streamList` cost 5 units per call, and
`insert` costs 20. Polling every 5 seconds would cost 3,600 units per chat hour. Prototype Google's
[`streamList`](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/streamList)
for chat. The connection delivers updates and reconnects can resume from the last page token. Measure
connection attempts, reconnects and messages sent before sizing quota. If polling is needed, honor the
API's polling interval and stop on broadcast end. More quota requires an audit
([Quota and Compliance Audits](https://developers.google.com/youtube/v3/guides/quota_and_compliance_audits)).
Ship go-live first. Request more quota if measured usage and expected traffic exceed the project's allowance.

**Draft justification (edit before use).**

> Zeno is a recording and live-streaming studio. A user who wants to broadcast to their own YouTube channel
> connects it once. Zeno then creates a live broadcast and stream on that channel, starts and ends it at the
> user's command. Those methods require a write-capable scope. Zeno requests `youtube.force-ssl` for
> these operations and limits its calls to the connected channel and broadcast.

Before filing, add the implemented token storage, retention and disconnect behavior to the
justification and privacy policy. Verify local credential deletion and Google revocation before
making those claims. Include chat in the submission only if it is implemented and shown in the demo.

**Demo video script (about 3 minutes).** 1. Sign in to Zeno. 2. Settings, Connect YouTube. 3. The Google
account chooser and the **full consent screen** (do not skip, do not cut). 4. Back in Zeno: the channel name
shown. 5. Create a stream, show the broadcast appearing in YouTube Studio, go live, end it. 6. Disconnect, and
show the access gone in the account's third-party access page.

## 2. Twitch

Registering an application costs nothing and the permissions are not entered at registration: they are named
in each sign-in request, and Twitch may suspend an app that asks for more than it uses
([Scopes](https://dev.twitch.tv/docs/authentication/scopes/),
[Register an app](https://dev.twitch.tv/docs/authentication/register-app)).

| Need | Scope to confirm in the scopes page before filing |
| --- | --- |
| Set the title and category of the stream | `channel:manage:broadcast` |
| Read the person's stream key so they do not paste it | `channel:read:stream_key` (documented for the Get Stream Key endpoint; leave it out if pasting a key is acceptable) |
| Read and write chat as the connected user | `user:read:chat` for EventSub and `user:write:chat` for Send Chat Message. [Twitch chat authentication](https://dev.twitch.tv/docs/chat/authenticating/) lists additional grants for a cloud bot. |

The [registration guide](https://dev.twitch.tv/docs/authentication/register-app/) requires the account
owner to enable **2FA**. Use the final app name, category and exact redirect URL. Create a secret only
if the OAuth flow needs it and store it outside this document.
Decision to make: whether the first version takes the stream key
with an OAuth scope at all. Pasting a key (as planned for X and Instagram) needs no review and no stored
token; the scope buys convenience only.

## 3. Windows code-signing

Read on 25 Nov 2026 from Microsoft Learn
([code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options),
[Artifact Signing FAQ](https://learn.microsoft.com/en-us/azure/artifact-signing/faq)):

| Option | Who can have it | Cost | Notes |
| --- | --- | --- | --- |
| Azure Artifact Signing (was Trusted Signing) | **Organisations in the USA, Canada, the EU and the UK; individuals only in the USA and Canada** | about 9.99 USD a month on the options page; the pricing page's tiers (5,000 or 100,000 signatures a month) did not show prices, so the figure is unconfirmed | needs a **paid** Azure subscription (no free, trial or sponsored); identity validation takes a few business days and cannot be expedited; the key never leaves the service; works from GitHub Actions; the certificate name must be the validated legal name |
| OV certificate from a certificate authority | anywhere | 150 to 300 USD a year (Microsoft's range) | the private key must be on an HSM or hardware token since June 2023, which makes CI signing need the vendor's cloud HSM or a token attached to a machine |
| EV certificate | anywhere | 400 USD and up a year | no longer gives instant trust: EV files build SmartScreen reputation the same way as the others; needed only for driver and hardware programs |

**Signing does not guarantee instant SmartScreen trust.** A consistent publisher can build reputation
across releases. Verify the signed installer on a clean Windows machine and report the observed result.

The decision to make: **Artifact Signing if the publisher (company or person) is in an eligible region and
Azure can be paid for; otherwise request an OV quote from a CA that accepts the publisher and supports
a cloud HSM reachable from CI.** Ask for total cost, validation requirements, cloud or hardware fees,
supported CI runners and renewal terms. The prices above are estimates.
[Electron-builder v26](https://www.electron.build/v26/docs/features/code-signing/code-signing-win/)
supports Azure signing through `win.azureSignOptions`. The [v27 docs](https://www.electron.build/docs/win/)
describe `win.sign` with `type: "azure"`. Use the configuration for the version pinned by Zeno and
verify the signed executable and installer with Authenticode.

## What only the owner can do

* Decide the **publisher**: a legal entity or an individual, and where (decides section 3 and the name the
  consent screen shows).
* Confirm the **production domain** and support email. The repo has a landing page. Filing still needs
  verified production URLs and a published privacy policy that reflects the implemented OAuth data,
  storage, retention and deletion behavior.
* Create the **Google Cloud project**, verify the domain, set the branding, add the scope, record the demo
  video, press submit. Create the **Twitch** application. Buy or start the **signing** identity validation.
* Nothing is to be filed with a placeholder name or domain: they are shown to the public and hard to change.

## Filing checklist

| Track | Next action | Evidence to record |
| --- | --- | --- |
| Publisher and domain | Confirm legal publisher, individual vs organisation, country, domain and support email. | Verified domain and final public publisher name. |
| Google branding | Create the Cloud project. Verify the domain, publish the homepage and privacy policy, set the exact HTTPS callback and complete branding. | Project ID, callback and policy URLs, branding status. |
| Google scope review | Implement and exercise Connect YouTube, go live and disconnect with a test channel. Confirm classification and capture the real consent screen and scope use. | Exact scope and classification, unlisted demo URL, submission reference or status. |
| Twitch | Register in parallel once 2FA is enabled and the redirect URL is decided. Validate that callback when the OAuth flow is built. | App name, public client ID and redirect URL. |
| Windows signing | Get a vendor quote or start Azure identity validation once publisher eligibility is clear. Verify the signature on a CI build before setting a distribution date. | Quote or validation reference, signer identity and signature verification result. |
| YouTube chat quota | Measure `streamList` and estimate peak daily demand. Request more quota if needed. | Measured usage, quota budget and audit reference if filed. |
