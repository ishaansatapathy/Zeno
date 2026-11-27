-- Recordings in a workspace (Day 33).
--
-- A recording is still its maker's: they keep it, edit it, share it and delete it. Putting it in a workspace
-- lets everyone in that workspace see it in the team's list and watch it, nothing more. Taking it out (by its
-- maker, or by an owner or admin of the workspace) ends that; the recording stays in its maker's library.
--
-- `null` is a personal recording, which is every recording until somebody puts it in a team. If the workspace
-- is purged the recording goes back to being personal rather than going with it.
alter table recordings
    add column workspace_id uuid references workspaces(id) on delete set null;

-- The team's list, newest first, in the same order and keyset as the library.
create index recordings_workspace_idx
    on recordings (workspace_id, created_at desc, id desc)
    where workspace_id is not null and deleted_at is null;

-- The single source of truth for "may this person watch this video as a person?" gains one way in: being in a
-- workspace that holds a recording made of it. Everything that asks (a viewing's start, every playlist and
-- every key, the sweep that ends viewings whose reason has gone) asks this, so being removed from the team, the
-- recording being taken out of it, the recording being deleted and the workspace being ended all end a viewing
-- at its next request, with no code of their own.
--
-- Nothing can reach the new branch until a recording has a workspace, so an old binary meeting the new
-- function sees no change.
create or replace function video_access_state(p_user_id uuid, p_video_id uuid)
returns text
language sql
stable
security invoker
set search_path = public
as $$
    select case
        when exists (
            select 1 from videos v join users u on u.id = p_user_id
            where v.id = p_video_id and (v.owner_id = p_user_id or u.role = 'admin')
        ) then 'allowed'
        when exists (
            select 1
            from recordings r
            join workspaces w on w.id = r.workspace_id and w.deleted_at is null
            join workspace_members m on m.workspace_id = w.id and m.user_id = p_user_id
            where r.video_id = p_video_id and r.deleted_at is null
        ) then 'allowed'
        when exists (
            select 1
            from lessons l
            join courses c on c.id = l.course_id
            where l.video_id = p_video_id
              and l.deleted_at is null
              and (
                  (
                      l.is_free_preview
                      and c.status = 'published'
                      and c.deleted_at is null
                  )
                  or exists (
                      select 1 from enrollments e
                      where e.course_id = c.id
                        and e.user_id = p_user_id
                        and e.revoked_at is null
                        and (e.expires_at is null or e.expires_at > now())
                        and (c.deleted_at is null or c.deleted_at > now() - interval '30 days')
                  )
              )
        ) then 'allowed'
        when exists (
            select 1
            from lessons l
            join courses c on c.id = l.course_id
            join enrollments e on e.course_id = c.id and e.user_id = p_user_id
            where l.video_id = p_video_id
              and l.deleted_at is null
              and e.revoked_at is null
              and e.expires_at <= now()
              and (c.deleted_at is null or c.deleted_at > now() - interval '30 days')
        ) then 'expired'
        else 'access_revoked'
    end
$$;
