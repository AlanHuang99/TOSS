-- Personal access token scopes.
--
-- `git` authorizes Git smart HTTP and `api` authorizes REST requests that send
-- the token as a bearer credential. Tokens created before scopes existed keep
-- Git-only access.

ALTER TABLE public.personal_access_tokens
    ADD COLUMN scopes text[] DEFAULT '{git}'::text[] NOT NULL;

ALTER TABLE public.personal_access_tokens
    ADD CONSTRAINT personal_access_tokens_scopes_check CHECK (
        cardinality(scopes) > 0
        AND scopes <@ ARRAY['git'::text, 'api'::text]
    );
