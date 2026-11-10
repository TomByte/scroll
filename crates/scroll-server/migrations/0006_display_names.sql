-- The name a sign-in provider gives for the person (Google's `name`), shown to others on their caret. Password
-- accounts have none; the email's local part stands in.
ALTER TABLE users ADD COLUMN display_name TEXT;
