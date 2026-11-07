using UnityEngine;

public class PlayerMovement : MonoBehaviour
{
    public float jumpHeight = 3f;

    private void Update()
    {
        if (Input.GetButtonDown("Jump")) Jump();
    }

    public void Jump()
    {
        Health.ApplyDamage(1);
    }
}
