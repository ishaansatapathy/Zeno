-- Content keys are now stored wrapped (AES-256-GCM, see zeno_core::keyvault), never raw.
-- Zeno has no production data yet: keys written in the old raw format cannot be wrapped from
-- SQL, so any that exist (development databases only) are dropped and their videos must be
-- uploaded again.
update videos set status = 'failed', error_code = 'key_reset', ready_at = null
    where id in (select video_id from video_keys);
delete from video_keys;

alter table video_keys drop constraint video_keys_key_check;
alter table video_keys rename column key to wrapped_key;
-- version (1) + nonce (12) + ciphertext (16) + tag (16)
alter table video_keys add constraint video_keys_wrapped_key_len check (octet_length(wrapped_key) = 45);
