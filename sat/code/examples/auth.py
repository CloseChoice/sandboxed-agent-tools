class Client:
    # Renew the cached token.
    def refresh(self):
        token = self.refresh_token
        return token

    def upload(self):
        return self.send("payload")
