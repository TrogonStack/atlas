// Where the browser's API key is entered.
//
// Shown when the bridge answers 401, which is the only moment the studio
// knows a credential is needed: with TROGON_ATLAS_AUTH_PASSTHROUGH off the
// bridge speaks for everyone and this never appears, and with it on the
// server decides, so asking up front would be guessing.
import { type FormEvent, useState } from 'react';
import { setCredential } from '@/lib/credential';

export function CredentialPrompt({ message, onDismiss }: { message?: string; onDismiss?: () => void }) {
  const [value, setValue] = useState('');

  function submit(e: FormEvent) {
    e.preventDefault();
    if (value.trim().length === 0) return;
    setCredential(value);
    setValue('');
    onDismiss?.();
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="credential-prompt-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
    >
      <form onSubmit={submit} className="flex w-full max-w-md flex-col gap-4 rounded-lg bg-card p-6 shadow-xl">
        <h2 id="credential-prompt-title" className="text-base font-semibold text-card-foreground">
          API key required
        </h2>
        <p className="text-sm text-muted-foreground">
          This studio forwards your key to the server, which decides what you can see. Paste the key you were issued.
        </p>
        {message ? (
          <p role="alert" className="text-sm text-red-700">
            {message}
          </p>
        ) : null}
        <label className="flex flex-col gap-1 text-sm text-foreground">
          <span>API key</span>
          <input
            type="password"
            autoComplete="off"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            className="rounded border border-input px-2 py-1 font-mono text-sm"
          />
        </label>
        <p className="text-xs text-muted-foreground">Kept for this tab only, and forgotten when you close it.</p>
        <button
          type="submit"
          disabled={value.trim().length === 0}
          className="self-end rounded bg-primary px-3 py-1.5 text-sm text-primary-foreground disabled:opacity-40"
        >
          Use this key
        </button>
      </form>
    </div>
  );
}
