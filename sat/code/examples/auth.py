class Client:
    # Renew the cached token.
    def refresh(self):
        """Return the cached refresh token."""
        token = self.refresh_token
        return token

    def upload(self):
        return self.send("payload")
