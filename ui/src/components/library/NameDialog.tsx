import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { nameError } from "@/lib/library";

interface Props {
  open: boolean;
  title: string;
  description: string;
  /** Filled in and selected, so typing replaces it. */
  initial: string;
  confirm: string;
  /** Gets the trimmed name; a rejection is shown under the field. */
  onSubmit: (name: string) => Promise<void> | void;
  onClose: () => void;
}

/** Asks for a clip's or session's name. */
export function NameDialog(props: Props) {
  return (
    <Dialog open={props.open} onOpenChange={(open) => !open && props.onClose()}>
      <DialogContent>
        <NameForm {...props} />
      </DialogContent>
    </Dialog>
  );
}

// Mounted with each opening, so it starts from `initial` every time.
function NameForm({ title, description, initial, confirm, onSubmit, onClose }: Props) {
  const [name, setName] = useState(initial);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  // What was suggested can always be kept.
  const error = name === initial ? null : nameError(name);
  const shown = failure ?? (name !== initial ? error : null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (error || saving) return;
    setSaving(true);
    try {
      await onSubmit(name.trim());
      onClose();
    } catch (err) {
      setFailure(String(err));
      setSaving(false);
    }
  };

  return (
    <form onSubmit={submit} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>{description}</DialogDescription>
      </DialogHeader>
      <div className="grid gap-1.5">
        <Input
          autoFocus
          aria-label="Name"
          value={name}
          aria-invalid={shown !== null}
          onFocus={(e) => e.currentTarget.select()}
          onChange={(e) => {
            setName(e.target.value);
            setFailure(null);
          }}
        />
        {shown && <p className="text-xs text-destructive">{shown}</p>}
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={error !== null || saving}>
          {confirm}
        </Button>
      </DialogFooter>
    </form>
  );
}
