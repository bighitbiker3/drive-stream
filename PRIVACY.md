# Privacy Policy

_Last updated: November 9, 2026_

Drive Stream is open-source software that runs on your own Mac. It mounts your Google Drive as a folder so files can be opened without downloading them in full. This policy describes what it does with your data.

## What Drive Stream accesses

With your permission (the Google Drive scope you grant when you sign in), Drive Stream reads and writes the files in your Google Drive so that you can browse, open, create, rename, move and delete them through the mounted folder.

## Where your data goes

- **Your Mac and Google, nothing else.** Drive Stream talks only to Google's own services (`accounts.google.com`, `oauth2.googleapis.com`, `www.googleapis.com`). The authors run no servers, and Drive Stream sends no data, analytics or telemetry to anyone.
- **Stored on your Mac:** your Google sign-in (a refresh token, in a file only your user account can read), a list of your Drive files' names and details, and a cache of file contents you've opened, all in `~/Library/Application Support/DriveStream`. Files waiting to upload are kept there until they're uploaded.
- **Not shared or sold.** No data is shared with, transferred to or sold to anyone. Data obtained through Google APIs is used only to provide Drive Stream's features to you.

Drive Stream's use of information received from Google APIs adheres to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including the Limited Use requirements.

## Deleting your data

- Run `./uninstall.sh --purge` to delete everything Drive Stream stored on your Mac, including your sign-in and cache.
- Revoke Drive Stream's access at any time at [myaccount.google.com/permissions](https://myaccount.google.com/permissions).

Your files themselves stay in your Google Drive; Drive Stream never deletes them permanently (deleting a file moves it to Drive's trash).

## Contact

Questions or concerns: open an issue at [github.com/bighitbiker3/drive-stream/issues](https://github.com/bighitbiker3/drive-stream/issues).
