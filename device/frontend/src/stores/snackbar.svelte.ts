class Snackbar {
  message = $state<string | null>(null);
  error = $state(false);
  #timer: ReturnType<typeof setTimeout> | null = null;

  show(message: string, error = false) {
    this.message = message;
    this.error = error;
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => (this.message = null), error ? 8000 : 5000);
  }

  dismiss() {
    this.message = null;
  }
}

export const snackbar = new Snackbar();
